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
    let mut settings = Settings::initial_settings(plugins)?;
    settings.reset_basic();
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
        assert_eq!(settings.microphone_id, "default://console");
        assert_eq!(settings.microphone_name, "既定のデバイス");
        assert_eq!(settings.cable_id, "default://console");
        assert_eq!(
            settings.rvc_parameters,
            crate::config::initial_rvc_parameters()
        );
        assert!(settings.voice_input_id.is_empty() && settings.monitor_id.is_empty());
        let ptt = &settings.hotkeys[13];
        assert!(ptt.is_assigned() && ptt.modifier_only && ptt.ctrl && ptt.shift);
        assert_eq!(ptt.ctrl_side, crate::config::ModifierSide::Left);
        assert_eq!(ptt.shift_side, crate::config::ModifierSide::Left);
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
