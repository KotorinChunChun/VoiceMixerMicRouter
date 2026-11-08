//! 常駐設定の型と検証。保存・候補選択・グラフ生成は用途別の子モジュールへ分離する。
#[path = "config/devices.rs"]
mod devices;
#[path = "config/hotkey.rs"]
mod hotkey;
#[path = "config/project.rs"]
mod project;
#[path = "config/storage.rs"]
mod storage;
#[cfg(test)]
#[path = "config/tests.rs"]
mod tests;
pub use crate::actions::{HOTKEY_COUNT, HOTKEY_LABELS};
use audio_mixer_backend::windows::devices::DEFAULT_CONSOLE_ENDPOINT;
#[cfg(test)]
pub use devices::choose_cable;
pub use devices::{device_choices, is_default_device};
use hotkey::deserialize_hotkeys;
pub use hotkey::{default_hotkeys, Hotkey, KeyHandling, ModifierSide};
pub use project::{OUTPUT_SELECTOR, SOURCE_SELECTOR, VOICE_INPUT_OUTPUT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
pub use storage::write_json;
pub const NOISE_PARAMETER: &str = "transient_noise_suppression";
/// 常駐ツール固有の版。本体Cargoパッケージとは独立して更新する。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "VoiceMixerMicRouter";
pub const UNSELECTED: &str = "未選択";
pub(crate) const BRIEF_DURATION: Duration = Duration::from_secs(4);
pub fn initial_rvc_parameters() -> Value {
    json!({
        "model": "models/KahoA.pth", "index": "models/Kaho.index", "pitch": 10.0,
        "f0_method": "rmvpe", "index_rate": 0.0, "block_time": 0.25,
        "crossfade_time": 0.08, "extra_time": 2.5, "rms_mix_rate": 0.5,
        "threshold_db": -58.0, "transient_noise_suppression": true
    })
}
pub fn default_switch_sounds() -> [u8; 7] {
    [1, 2, 3, 4, 5, 6, 7]
}
pub fn default_output_labels() -> [String; 3] {
    ["配信用", "音声入力用", "モニタ用"].map(String::from)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Direct,
    Rvc,
    SpeakerRvc,
    Muted,
}
impl Mode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "direct" => Ok(Self::Direct),
            "rvc" => Ok(Self::Rvc),
            "speaker_rvc" | "spk-rvc" => Ok(Self::SpeakerRvc),
            "muted" => Ok(Self::Muted),
            _ => Err("モードは direct、rvc、speaker_rvc、muted です".into()),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "MIC",
            Self::Rvc => "MIC-RVC",
            Self::SpeakerRvc => "SPK-RVC",
            Self::Muted => "送信停止",
        }
    }
    pub fn sound_index(self) -> usize {
        match self {
            Self::Direct => 0,
            Self::Rvc => 1,
            Self::SpeakerRvc => 2,
            Self::Muted => 3,
        }
    }
    pub fn other(self) -> Self {
        match self {
            Self::Direct => Self::Rvc,
            Self::Rvc | Self::SpeakerRvc | Self::Muted => Self::Direct,
        }
    }
    pub fn is_rvc(self) -> bool {
        matches!(self, Self::Rvc | Self::SpeakerRvc)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMode {
    Off,
    Brief,
    #[default]
    Always,
}
impl OverlayMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Brief, Self::Always];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "オフ",
            Self::Brief => "数秒表示",
            Self::Always => "常時表示",
        }
    }
    pub fn visible_after(self, elapsed: Duration) -> bool {
        self == Self::Always || (self == Self::Brief && elapsed < BRIEF_DURATION)
    }
    pub fn brief_remaining(self, elapsed: Duration) -> Option<u8> {
        (self == Self::Brief && self.visible_after(elapsed))
            .then(|| (BRIEF_DURATION.as_secs() - elapsed.as_secs()) as u8)
    }
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Brief,
            Self::Brief => Self::Always,
            Self::Always => Self::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub device_selection_revision: u8,
    #[serde(default)]
    pub hotkey_defaults_revision: u8,
    #[serde(default = "enabled_default")]
    pub rvc_enabled: bool,
    /// 入力4モードと出力3系統それぞれの効果音。0はなし。
    #[serde(default = "default_switch_sounds")]
    pub switch_sounds: [u8; 7],
    #[serde(default = "default_console_id")]
    pub sound_device_id: String,
    #[serde(default = "default_console_name")]
    pub sound_device_name: String,
    /// 表示専用の用途名。デバイス名や音声ノードの識別には使わない。
    #[serde(default = "default_output_labels")]
    pub output_labels: [String; 3],
    pub microphone_id: String,
    pub microphone_name: String,
    #[serde(default = "default_console_id")]
    pub speaker_id: String,
    #[serde(default = "default_console_name")]
    pub speaker_name: String,
    pub cable_id: String,
    pub cable_name: String,
    #[serde(default = "enabled_default")]
    pub cable_enabled: bool,
    #[serde(default = "default_voice_input_id")]
    pub voice_input_id: String,
    #[serde(default = "default_voice_input_name")]
    pub voice_input_name: String,
    #[serde(default)]
    pub voice_input_enabled: bool,
    pub plugin_directory: PathBuf,
    pub rvc_parameters: Value,
    pub last_mode: Mode,
    /// None は前回の成功モードを復元。
    pub startup_mode: Option<Mode>,
    #[serde(default, skip_serializing)]
    pub hotkey: String,
    #[serde(
        default = "hotkey::legacy_default_hotkeys",
        deserialize_with = "deserialize_hotkeys"
    )]
    pub hotkeys: [Hotkey; HOTKEY_COUNT],
    #[serde(default)]
    pub overlay_mode: OverlayMode,
    #[serde(default)]
    pub monitoring: bool,
    #[serde(default)]
    pub monitor_id: String,
    #[serde(default)]
    pub monitor_name: String,
}

fn default_console_id() -> String {
    DEFAULT_CONSOLE_ENDPOINT.into()
}
fn enabled_default() -> bool {
    true
}
fn default_voice_input_id() -> String {
    "unavailable://cable-d".into()
}
fn default_voice_input_name() -> String {
    "VB-CABLE-D".into()
}
fn default_console_name() -> String {
    "既定のデバイス".into()
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DeviceSelection {
    pub microphone: bool,
    pub speaker: bool,
    pub outputs: [bool; 3],
}
impl Default for DeviceSelection {
    fn default() -> Self {
        Self {
            microphone: true,
            speaker: true,
            outputs: [true; 3],
        }
    }
}
impl DeviceSelection {
    pub fn filter_holds(self, holds: [bool; 3]) -> [bool; 3] {
        std::array::from_fn(|i| holds[i] && self.outputs[i])
    }
    pub fn has_source(self, mode: Mode) -> bool {
        match mode {
            Mode::Direct | Mode::Rvc => self.microphone,
            Mode::SpeakerRvc => self.speaker,
            Mode::Muted => true,
        }
    }
    pub fn permits(self, action: usize) -> bool {
        use crate::actions::*;
        match action {
            DIRECT | RVC | TOGGLE => self.microphone,
            SPK_RVC => self.speaker,
            OUTPUT1 | PTT1 => self.outputs[0],
            OUTPUT2 | PTT2 => self.outputs[1],
            MONITOR | PTT3 => self.outputs[2],
            _ => true,
        }
    }
}
impl Settings {
    pub fn reset_basic(&mut self) {
        self.overlay_mode = OverlayMode::default();
        self.startup_mode = None;
        self.rvc_enabled = false;
        self.output_labels = default_output_labels();
        (self.microphone_id, self.microphone_name) = (default_console_id(), default_console_name());
        (self.speaker_id, self.speaker_name) = (default_console_id(), default_console_name());
        (self.cable_id, self.cable_name) = (default_console_id(), default_console_name());
        (self.voice_input_id, self.voice_input_name) = (String::new(), UNSELECTED.into());
        (self.monitor_id, self.monitor_name) = (String::new(), UNSELECTED.into());
        self.cable_enabled = false;
        self.voice_input_enabled = false;
        self.monitoring = false;
        self.rvc_parameters[NOISE_PARAMETER] = initial_rvc_parameters()[NOISE_PARAMETER].clone();
    }

    pub fn reset_sounds(&mut self) {
        self.switch_sounds = default_switch_sounds();
        self.sound_device_id = default_console_id();
        self.sound_device_name = default_console_name();
    }

    pub fn output_flags(&self) -> [bool; 3] {
        [self.cable_enabled, self.voice_input_enabled, self.monitoring]
    }
    pub fn toggle_output(&mut self, index: usize) {
        let flag = match index {
            0 => &mut self.cable_enabled,
            1 => &mut self.voice_input_enabled,
            2 => &mut self.monitoring,
            _ => unreachable!("出力番号は事前に検証する"),
        };
        *flag = !*flag;
    }
    pub fn device_selection(&self) -> DeviceSelection {
        DeviceSelection {
            microphone: !self.microphone_id.is_empty(),
            speaker: !self.speaker_id.is_empty(),
            outputs: [&self.cable_id, &self.voice_input_id, &self.monitor_id]
                .map(|id| !id.is_empty()),
        }
    }
    pub fn effective_startup_mode(&self, initial: Option<Mode>) -> Mode {
        let requested = initial.or(self.startup_mode).unwrap_or(self.last_mode);
        if !self.rvc_enabled && requested.is_rvc() {
            Mode::Muted
        } else {
            self.available_mode(requested)
        }
    }
    pub fn available_mode(&self, mode: Mode) -> Mode {
        let devices = self.device_selection();
        if !devices.has_source(mode) {
            Mode::Muted
        } else if !self.rvc_enabled && mode.is_rvc() && devices.microphone {
            Mode::Direct
        } else if !self.rvc_enabled && mode.is_rvc() {
            Mode::Muted
        } else {
            mode
        }
    }
    /// 押下中の出力だけを有効化する。保存対象の設定値は変更しない。
    pub fn with_output_holds(&self, holds: [bool; 3]) -> Self {
        let holds = self.device_selection().filter_holds(holds);
        let mut effective = self.clone();
        if holds.iter().any(|held| *held) {
            effective.cable_enabled = holds[0];
            effective.voice_input_enabled = holds[1];
            effective.monitoring = holds[2];
        }
        effective
    }

    pub fn output_label(&self, index: usize) -> String {
        format!("出力{}（{}）", index + 1, self.output_labels[index])
    }
    pub fn noise_suppression(&self) -> bool {
        self.rvc_parameters
            .get(NOISE_PARAMETER)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub fn set_noise_suppression(&mut self, enabled: bool) {
        self.rvc_parameters[NOISE_PARAMETER] = json!(enabled);
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.switch_sounds.iter().any(|sound| *sound > 7) {
            return Err("効果音の選択が不正です".into());
        }
        if !self.microphone_id.is_empty()
            && self.microphone_id == self.cable_id
            && !is_default_device(&self.microphone_id)
        {
            return Err("マイクとVB-CABLEの異なるEndpoint IDが必要です".into());
        }
        if !self.plugin_directory.is_absolute() {
            return Err("plugin_directoryは絶対パスで指定してください".into());
        }
        if !self.rvc_parameters.is_object() {
            return Err("rvc_parametersはJSONオブジェクトです".into());
        }
        if self
            .rvc_parameters
            .get(NOISE_PARAMETER)
            .is_some_and(|value| !value.is_boolean())
        {
            return Err("短いノイズの抑制はONまたはOFFで指定してください".into());
        }
        for label in &self.output_labels {
            if label.trim().is_empty()
                || label.chars().count() > 24
                || label.chars().any(char::is_control)
            {
                return Err(
                    "出力の用途名は空白だけを除く1〜24文字で指定してください（改行不可）".into(),
                );
            }
        }
        let mut assigned: Vec<&Hotkey> = Vec::new();
        for (index, key) in self.hotkeys.iter().enumerate() {
            if key.key == "None" && !key.modifier_only {
                continue;
            }
            if key.key == "None" {
                if crate::actions::ptt_event(crate::actions::HOTKEY_COMMANDS[index]).is_none()
                    || key.modifiers() == 0
                    || key.handling != KeyHandling::Coexist
                {
                    return Err("修飾キー単独は押している間ONに共存で割り当ててください".into());
                }
            } else {
                key.virtual_key()?;
            }
            if assigned.iter().any(|other| key.conflicts_with(other)) {
                return Err("同じショートカットを複数の操作へ割り当てることはできません".into());
            }
            assigned.push(key);
        }
        let mut outputs = std::collections::HashSet::new();
        for (on, id) in [
            (self.cable_enabled, &self.cable_id),
            (self.voice_input_enabled, &self.voice_input_id),
            (self.monitoring, &self.monitor_id),
        ] {
            if on && (id.is_empty() || id.starts_with("unavailable://")) {
                return Err("ONにする出力のデバイスを選択してください".into());
            }
            if on && !outputs.insert(id) {
                return Err("ONにする出力にはそれぞれ異なるデバイスを選択してください".into());
            }
        }
        Ok(())
    }
}
