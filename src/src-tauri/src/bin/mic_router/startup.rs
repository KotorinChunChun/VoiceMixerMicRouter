//! 初回導入と従来のEXE隣設定の互換。利用者の既存設定は上書きしない。
use crate::config::{Mode, Settings};
use std::path::{Path, PathBuf};
pub fn default_config(exe: &Path, local: &Path) -> PathBuf {
    let previous = exe.with_file_name("mic-settings.json");
    if previous.is_file() {
        previous
    } else {
        local.join("VoiceMixerMicRouter/mic-settings.json")
    }
}
pub fn initialize(path: &Path, plugins: &Path) -> Result<bool, String> {
    if path.exists() {
        return Ok(false);
    }
    let mut settings = Settings::detect(plugins).or_else(|_| {
        serde_json::from_value::<Settings>(serde_json::json!({
            "microphone_id":"default://console", "microphone_name":"既定のデバイス",
            "cable_id":"default://console", "cable_name":"既定のデバイス",
            "cable_enabled":false, "voice_input_enabled":false, "monitoring":false,
            "plugin_directory":std::fs::canonicalize(plugins).map_err(|e|e.to_string())?,
            "rvc_parameters":{},"last_mode":"muted","hotkey_defaults_revision":1
        }))
        .map_err(|e| e.to_string())
    })?;
    settings.rvc_enabled = false;
    settings.cable_enabled = false;
    settings.voice_input_enabled = false;
    settings.monitoring = false;
    settings.last_mode = Mode::Muted;
    settings.save(path)?;
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_run_is_muted_and_existing_settings_are_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        let config = dir.path().join("user/mic-settings.json");
        assert!(initialize(&config, &plugins).unwrap());
        let settings = Settings::load(&config).unwrap();
        assert!(
            !settings.rvc_enabled
                && !settings.cable_enabled
                && !settings.monitoring
                && !settings.voice_input_enabled
        );
        assert_eq!(settings.last_mode, Mode::Muted);
        let before = std::fs::read(&config).unwrap();
        assert!(!initialize(&config, &plugins).unwrap());
        assert_eq!(std::fs::read(&config).unwrap(), before);
    }
    #[test]
    fn old_sibling_config_precedes_new_user_directory() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("VoiceMixerMicRouter.exe");
        let user = dir.path().join("data");
        assert_eq!(
            default_config(&exe, &user),
            user.join("VoiceMixerMicRouter/mic-settings.json")
        );
        std::fs::write(exe.with_file_name("mic-settings.json"), "existing").unwrap();
        assert_eq!(
            default_config(&exe, &user),
            exe.with_file_name("mic-settings.json")
        );
    }
}
