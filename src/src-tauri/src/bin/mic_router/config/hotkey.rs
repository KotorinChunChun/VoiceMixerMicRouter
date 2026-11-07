//! キー設定の保存順、既定値と旧版移行。
use crate::actions::{HOTKEY_COUNT, SHORTCUTS};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyHandling {
    #[default]
    Coexist,
    Override,
}
impl KeyHandling {
    pub const ALL: [Self; 2] = [Self::Coexist, Self::Override];
    pub fn label(self) -> &'static str {
        match self {
            Self::Coexist => "共存",
            Self::Override => "上書き",
        }
    }
}

/// 左右を区別しなかった従来の true は Either として扱う。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModifierSide {
    #[default]
    Either,
    Left,
    Right,
    Both,
}

impl ModifierSide {
    pub fn matches(self, left: bool, right: bool) -> bool {
        match self {
            Self::Either => left || right,
            Self::Left => left,
            Self::Right => right,
            Self::Both => left && right,
        }
    }
    /// 主キー押下時は左右を区別して重複割当を判定する。
    pub fn matches_exact(self, left: bool, right: bool) -> bool {
        match self {
            Self::Left => left && !right,
            Self::Right => !left && right,
            _ => self.matches(left, right),
        }
    }
}

fn is_either(side: &ModifierSide) -> bool {
    *side == ModifierSide::Either
}
fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    #[serde(default)]
    pub handling: KeyHandling,
    pub enabled: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub win: bool,
    pub alt: bool,
    pub key: String,
    #[serde(default, skip_serializing_if = "is_either")]
    pub shift_side: ModifierSide,
    #[serde(default, skip_serializing_if = "is_either")]
    pub ctrl_side: ModifierSide,
    #[serde(default, skip_serializing_if = "is_either")]
    pub win_side: ModifierSide,
    #[serde(default, skip_serializing_if = "is_either")]
    pub alt_side: ModifierSide,
    /// 新形式のP2Tだけに使用する。旧設定の有効・主キーなしは未割当のまま保つ。
    #[serde(default, skip_serializing_if = "is_false")]
    pub modifier_only: bool,
}
impl Hotkey {
    pub fn is_assigned(&self) -> bool {
        self.enabled && (self.key != "None" || (self.modifier_only && self.modifiers() != 0))
    }
    pub fn key_label(&self) -> &str {
        if self.is_assigned() {
            &self.key
        } else {
            "なし"
        }
    }
    pub fn virtual_key(&self) -> Result<u32, String> {
        let s = self.key.as_bytes();
        if s.len() == 1 && s[0].is_ascii_uppercase() {
            return Ok(s[0] as u32);
        }
        if let Some(number) = self
            .key
            .strip_prefix('F')
            .and_then(|s| s.parse::<u32>().ok())
        {
            if (1..=24).contains(&number) && self.key == format!("F{number}") {
                return Ok(0x70 + number - 1);
            }
        }
        Err("キーはA〜ZまたはF1〜F24から選択してください".into())
    }
    pub fn modifiers(&self) -> u32 {
        self.alt as u32
            | ((self.ctrl as u32) << 1)
            | ((self.shift as u32) << 2)
            | ((self.win as u32) << 3)
    }
    /// ビット順は Alt、Ctrl、Shift、Windows。
    pub fn sides(&self) -> [ModifierSide; 4] {
        [
            self.alt_side,
            self.ctrl_side,
            self.shift_side,
            self.win_side,
        ]
    }
    pub fn conflicts_with(&self, other: &Self) -> bool {
        if !self.is_assigned() || !other.is_assigned() || self.key != other.key {
            return false;
        }
        let a = self.modifiers();
        let b = other.modifiers();
        if a != b {
            return false;
        }
        for i in 0..4 {
            if a & (1 << i) == 0 {
                continue;
            }
            let left = self.sides()[i];
            let right = other.sides()[i];
            if self.key == "None" {
                if left != right {
                    return false;
                }
            } else if !(1..=3).any(|state| {
                left.matches_exact(state & 1 != 0, state & 2 != 0)
                    && right.matches_exact(state & 1 != 0, state & 2 != 0)
            }) {
                return false;
            }
        }
        true
    }
    pub fn label(&self) -> String {
        if !self.is_assigned() {
            return "割当なし".into();
        }
        let mut labels = Vec::new();
        for (on, side, label) in [
            (self.ctrl, self.ctrl_side, "Ctrl"),
            (self.shift, self.shift_side, "Shift"),
            (self.alt, self.alt_side, "Alt"),
            (self.win, self.win_side, "Win"),
        ] {
            if on {
                labels.push(match side {
                    ModifierSide::Either => label.to_owned(),
                    ModifierSide::Left => format!("左{label}"),
                    ModifierSide::Right => format!("右{label}"),
                    ModifierSide::Both => format!("左&右{label}"),
                });
            }
        }
        if self.key != "None" {
            labels.push(self.key.clone());
        }
        labels.join("+")
    }
}
pub(super) fn legacy_default_hotkeys() -> [Hotkey; HOTKEY_COUNT] {
    std::array::from_fn(|i| {
        let key = SHORTCUTS[i].default_key;
        Hotkey {
            handling: KeyHandling::Coexist,
            enabled: key != "None",
            shift: key != "None",
            ctrl: false,
            win: false,
            alt: false,
            key: key.into(),
            shift_side: ModifierSide::Either,
            ctrl_side: ModifierSide::Either,
            win_side: ModifierSide::Either,
            alt_side: ModifierSide::Either,
            modifier_only: false,
        }
    })
}
pub fn default_hotkeys() -> [Hotkey; HOTKEY_COUNT] {
    let mut keys = legacy_default_hotkeys();
    let output3 = &mut keys[13];
    output3.enabled = true;
    output3.ctrl = true;
    output3.shift = true;
    output3.ctrl_side = ModifierSide::Left;
    output3.shift_side = ModifierSide::Left;
    output3.modifier_only = true;
    keys
}
pub(super) fn deserialize_hotkeys<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<[Hotkey; HOTKEY_COUNT], D::Error> {
    let keys = Vec::<Hotkey>::deserialize(deserializer)?;
    if !matches!(keys.len(), 3 | 6 | 7 | 8 | 9 | 11 | HOTKEY_COUNT) {
        return Err(serde::de::Error::custom(
            "キー設定は旧3行・6行・7行・8行・9行・11行または14行で指定してください",
        ));
    }
    // 旧形式の追加既定を先に復元する。Shiftへの変更はSettings::loadで一度だけ行う。
    let mut result = legacy_default_hotkeys();
    if keys.len() < HOTKEY_COUNT {
        for key in &mut result {
            key.shift = false;
            key.alt = key.key != "None";
        }
    }
    let old_count = keys.len();
    for (i, key) in keys.into_iter().enumerate() {
        result[i] = key;
    }
    if old_count < 11 {
        // 旧表示の既定キーだけを移す。既存の独自割当との競合では新規側を未割当とする。
        let mut changed = std::array::from_fn::<_, HOTKEY_COUNT, _>(|i| i >= old_count);
        for (i, old, new) in [
            (
                4,
                if old_count == 6 {
                    "F7"
                } else if old_count < 8 {
                    "F9"
                } else {
                    "F10"
                },
                "F12",
            ),
            (5, if old_count < 8 { "F8" } else { "F9" }, "F11"),
            (6, "F7", "F8"),
        ] {
            if i < old_count
                && (i != 6 || old_count < 8)
                && result[i].enabled
                && result[i].modifiers() == 1
                && result[i].key == old
            {
                result[i].key = new.into();
                changed[i] = true;
            }
        }
        for i in 0..HOTKEY_COUNT {
            if changed[i]
                && result[i].enabled
                && (0..HOTKEY_COUNT).any(|j| {
                    j != i
                        && result[j].enabled
                        && (!changed[j] || j < i)
                        && result[j].modifiers() == result[i].modifiers()
                        && result[j].key == result[i].key
                })
            {
                result[i].enabled = false;
            }
        }
    }
    Ok(result)
}

pub(super) fn migrate_shift_defaults(settings: &mut super::Settings) {
    if settings.hotkey_defaults_revision >= 1 {
        return;
    }
    let previous = settings.hotkeys.clone();
    for (i, key) in settings.hotkeys.iter_mut().enumerate() {
        let default_key = SHORTCUTS[i].default_key;
        if key.is_assigned()
            && default_key != "None"
            && key.key == default_key
            && key.modifiers() == 1
        {
            // 独自のShift割当とぶつかる旧既定はそのまま残す。
            if !previous.iter().enumerate().any(|(j, other)| {
                j != i && other.is_assigned() && other.key == key.key && other.modifiers() == 4
            }) {
                key.alt = false;
                key.shift = true;
            }
        }
    }
    settings.hotkey_defaults_revision = 1;
}
