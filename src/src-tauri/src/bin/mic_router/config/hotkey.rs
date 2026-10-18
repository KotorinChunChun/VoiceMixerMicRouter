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
}
impl Hotkey {
    pub fn is_assigned(&self) -> bool {
        self.enabled && self.key != "None"
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
    pub fn label(&self) -> String {
        if !self.is_assigned() {
            return "割当なし".into();
        }
        let mut labels = Vec::new();
        for (on, label) in [
            (self.ctrl, "Ctrl"),
            (self.shift, "Shift"),
            (self.alt, "Alt"),
            (self.win, "Win"),
        ] {
            if on {
                labels.push(label);
            }
        }
        labels.push(&self.key);
        labels.join("+")
    }
}
pub fn default_hotkeys() -> [Hotkey; HOTKEY_COUNT] {
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
        }
    })
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
    let mut result = default_hotkeys();
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
