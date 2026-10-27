//! 設定の検出、旧版移行、読み書き。
use super::devices::choose_voice_input;
use super::{
    choose_cable, choose_microphone, default_hotkeys, default_speaker_id, default_speaker_name,
    default_voice_input_id, default_voice_input_name, Mode, OverlayMode, Settings,
};
use audio_mixer_backend::windows::devices::{
    list_devices, DeviceDirection, DEFAULT_CONSOLE_ENDPOINT,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{fs, path::Path};
impl Settings {
    pub fn detect(plugin_directory: &Path) -> Result<Self, String> {
        let inputs = list_devices(DeviceDirection::Capture).map_err(|e| e.to_string())?;
        let outputs = list_devices(DeviceDirection::Render).map_err(|e| e.to_string())?;
        let mic = choose_microphone(&inputs)?;
        let cable = choose_cable(&outputs)?;
        Ok(Self {
            hotkey_defaults_revision: 1,
            rvc_enabled: true,
            switch_sounds: super::default_switch_sounds(),
            sound_device_id: "default://console".into(),
            sound_device_name: "既定のデバイス".into(),
            output_labels: super::default_output_labels(),
            microphone_id: mic.endpoint_id.clone(),
            microphone_name: mic.friendly_name.clone(),
            speaker_id: default_speaker_id(),
            speaker_name: default_speaker_name(),
            cable_id: cable.endpoint_id.clone(),
            cable_name: cable.friendly_name.clone(),
            cable_enabled: true,
            voice_input_id: choose_voice_input(&outputs)
                .map(|d| d.endpoint_id.clone())
                .unwrap_or_else(default_voice_input_id),
            voice_input_name: choose_voice_input(&outputs)
                .map(|d| d.friendly_name.clone())
                .unwrap_or_else(default_voice_input_name),
            voice_input_enabled: false,
            plugin_directory: fs::canonicalize(plugin_directory).map_err(|e| e.to_string())?,
            rvc_parameters: json!({"model":"models/KahoA.pth", "index":"models/Kaho.index", "pitch":10.0,
                "f0_method":"rmvpe", "index_rate":0.0, "block_time":0.25, "crossfade_time":0.08,
                "extra_time":2.5, "rms_mix_rate":0.5, "threshold_db":-58.0, "transient_noise_suppression":true}),
            last_mode: Mode::Direct,
            startup_mode: None,
            hotkey: "割当なし".into(),
            hotkeys: default_hotkeys(),
            overlay_mode: OverlayMode::Always,
            monitoring: false,
            monitor_id: outputs
                .iter()
                .find(|d| d.is_default)
                .map(|d| d.endpoint_id.clone())
                .unwrap_or_default(),
            monitor_name: outputs
                .iter()
                .find(|d| d.is_default)
                .map(|d| d.friendly_name.clone())
                .unwrap_or_default(),
        })
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut source: Value = serde_json::from_slice(
            &fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?,
        )
        .map_err(|e| e.to_string())?;
        if source.get("overlay_mode").is_none() {
            if let Some(old) = source.get("overlay_always").and_then(Value::as_bool) {
                source["overlay_mode"] = json!(if old {
                    OverlayMode::Always
                } else {
                    OverlayMode::Brief
                });
            }
        }
        let mut value: Self = serde_json::from_value(source.clone()).map_err(|e| e.to_string())?;
        if value.voice_input_id == default_voice_input_id() {
            if let Ok(outputs) = list_devices(DeviceDirection::Render) {
                if let Some(device) = choose_voice_input(&outputs) {
                    value.voice_input_id = device.endpoint_id.clone();
                    value.voice_input_name = device.friendly_name.clone();
                }
            }
        }
        if source.get("hotkeys").is_none() && source.get("hotkey").is_some() {
            value.hotkeys[0].enabled = false;
            value.hotkeys[1].enabled = false;
            value.hotkeys[2].enabled = value.hotkey != "None";
            value.hotkeys[2].key = if value.hotkeys[2].enabled {
                "V"
            } else {
                "None"
            }
            .into();
            value.hotkeys[2].ctrl = value.hotkey.contains("Ctrl");
            value.hotkeys[2].shift = value.hotkey.contains("Shift");
            value.hotkeys[2].alt = value.hotkey.contains("Alt");
        }
        if value.monitor_id.is_empty() {
            value.monitor_id = DEFAULT_CONSOLE_ENDPOINT.into();
            value.monitor_name = "既定のデバイス".into();
        }
        value.hotkey = value.hotkeys[2].label();
        super::hotkey::migrate_shift_defaults(&mut value);
        value.validate()?;
        if source
            .get("hotkey_defaults_revision")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            < 1
        {
            value.save(path)?;
        }
        Ok(value)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        write_json(path, self)
    }
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}
