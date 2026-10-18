//! 操作IDとキー設定の対応。IPC番号・保存順は旧版との互換契約なので変更しない。
pub const DIRECT: usize = 1;
pub const RVC: usize = 2;
pub const TOGGLE: usize = 3;
pub const EXIT: usize = 4;
pub const RETRY: usize = 5;
pub const OVERLAY: usize = 6;
pub const SETTINGS: usize = 7;
pub const MONITOR: usize = 8;
pub const RELEASE_RVC: usize = 9;
pub const MUTED: usize = 10;
pub const NOISE: usize = 11;
pub const SPK_RVC: usize = 12;
pub const OUTPUT1: usize = 13;
pub const OUTPUT2: usize = 14;
pub const PTT1: usize = 30;
pub const PTT2: usize = 31;
pub const PTT3: usize = 32;
pub const PTT_RELEASE_OFFSET: usize = 3;
pub fn ptt_event(command: usize) -> Option<(usize, bool)> {
    match command {
        PTT1..=PTT3 => Some((command - PTT1, true)),
        33..=35 => Some((command - PTT1 - PTT_RELEASE_OFFSET, false)),
        _ => None,
    }
}
pub const SHOW: usize = 102;

pub struct ShortcutDefinition {
    pub command: usize,
    pub label: &'static str,
    pub default_key: &'static str,
}

impl ShortcutDefinition {
    const fn new(command: usize, label: &'static str, default_key: &'static str) -> Self {
        Self {
            command,
            label,
            default_key,
        }
    }
}

// mic-settings.json のhotkeys配列順。表示順は下記DISPLAY_ORDERで別に定義する。
pub const SHORTCUTS: [ShortcutDefinition; 14] = [
    ShortcutDefinition::new(DIRECT, "MICへ切替", "F5"),
    ShortcutDefinition::new(RVC, "MIC-RVCへ切替", "F6"),
    ShortcutDefinition::new(TOGGLE, "入力モードを交互に切替", "None"),
    ShortcutDefinition::new(SETTINGS, "設定画面を開く", "None"),
    ShortcutDefinition::new(OVERLAY, "オーバーレイ表示切替", "F12"),
    ShortcutDefinition::new(MONITOR, "出力3（モニタ用）切替", "F11"),
    ShortcutDefinition::new(MUTED, "送信停止へ切替", "F8"),
    ShortcutDefinition::new(SPK_RVC, "SPK-RVCへ切替", "F7"),
    ShortcutDefinition::new(NOISE, "ノイズ抑制切替", "None"),
    ShortcutDefinition::new(OUTPUT1, "出力1（配信用）切替", "F9"),
    ShortcutDefinition::new(OUTPUT2, "出力2（音声入力用）切替", "F10"),
    ShortcutDefinition::new(PTT1, "出力1 押している間ON", "None"),
    ShortcutDefinition::new(PTT2, "出力2 押している間ON", "None"),
    ShortcutDefinition::new(PTT3, "出力3 押している間ON", "None"),
];
pub const HOTKEY_COUNT: usize = SHORTCUTS.len();
pub const HOTKEY_LABELS: [&str; HOTKEY_COUNT] = {
    let mut labels = [""; HOTKEY_COUNT];
    let mut i = 0;
    while i < HOTKEY_COUNT {
        labels[i] = SHORTCUTS[i].label;
        i += 1;
    }
    labels
};
pub const HOTKEY_COMMANDS: [usize; HOTKEY_COUNT] = {
    let mut commands = [0; HOTKEY_COUNT];
    let mut i = 0;
    while i < HOTKEY_COUNT {
        commands[i] = SHORTCUTS[i].command;
        i += 1;
    }
    commands
};

pub fn shortcut_index(command: usize) -> Option<usize> {
    SHORTCUTS
        .iter()
        .position(|shortcut| shortcut.command == command)
}

pub fn shortcut_key(keys: &[String; HOTKEY_COUNT], command: usize) -> &str {
    shortcut_index(command).map_or("なし", |index| keys[index].as_str())
}

pub const DISPLAY_ORDER: [usize; HOTKEY_COUNT] = [
    OVERLAY, NOISE, SETTINGS, DIRECT, RVC, SPK_RVC, MUTED, TOGGLE, OUTPUT1, PTT1, OUTPUT2, PTT2,
    MONITOR, PTT3,
];

pub fn display_order() -> [usize; HOTKEY_COUNT] {
    DISPLAY_ORDER.map(|command| shortcut_index(command).expect("表示操作にはキー設定が必要です"))
}

/// 未割当は角括弧ごと省略する（設定画面の「なし」は維持）。
pub fn shortcut_badge(keys: &[String; HOTKEY_COUNT], command: usize) -> String {
    match shortcut_key(keys, command) {
        "なし" | "None" | "" => String::new(),
        key => format!("[{key}]"),
    }
}
