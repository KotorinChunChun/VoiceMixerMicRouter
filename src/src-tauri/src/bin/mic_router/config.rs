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
pub use devices::{choose_cable, choose_microphone, device_choices, is_default_device};
use hotkey::deserialize_hotkeys;
pub use hotkey::{default_hotkeys, Hotkey, KeyHandling};
pub use project::{OUTPUT_SELECTOR, SOURCE_SELECTOR, VOICE_INPUT_OUTPUT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
pub use storage::write_json;
pub const NOISE_PARAMETER: &str = "transient_noise_suppression";
/// 常駐ツール固有の版。本体Cargoパッケージとは独立して更新する。
pub const APP_VERSION: &str = "1.0.16";
pub const APP_NAME: &str = "VoiceMixerMicRouter";
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
    pub hotkey_defaults_revision: u8,
    #[serde(default = "enabled_default")]
    pub rvc_enabled: bool,
    /// 表示専用の用途名。デバイス名や音声ノードの識別には使わない。
    #[serde(default = "default_output_labels")]
    pub output_labels: [String; 3],
    pub microphone_id: String,
    pub microphone_name: String,
    #[serde(default = "default_speaker_id")]
    pub speaker_id: String,
    #[serde(default = "default_speaker_name")]
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
    #[serde(default = "default_hotkeys", deserialize_with = "deserialize_hotkeys")]
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

fn default_speaker_id() -> String {
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
fn default_speaker_name() -> String {
    "既定のデバイス".into()
}
impl Settings {
    /// RVCを停止している間も保存済みの起動モード・キー割当は保持する。
    pub fn available_mode(&self, mode: Mode) -> Mode {
        if !self.rvc_enabled && mode.is_rvc() {
            Mode::Direct
        } else {
            mode
        }
    }
    /// 押下中の出力だけを有効化する。保存対象の設定値は変更しない。
    pub fn with_output_holds(&self, holds: [bool; 3]) -> Self {
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
        if self.microphone_id.is_empty()
            || self.speaker_id.is_empty()
            || self.cable_id.is_empty()
            || (self.microphone_id == self.cable_id && !is_default_device(&self.microphone_id))
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
        let mut assigned = std::collections::HashSet::new();
        for key in &self.hotkeys {
            if key.key == "None" {
                continue;
            }
            let vk = key.virtual_key()?;
            if key.enabled && !assigned.insert((key.modifiers(), vk)) {
                return Err("同じショートカットを複数の操作へ割り当てることはできません".into());
            }
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
