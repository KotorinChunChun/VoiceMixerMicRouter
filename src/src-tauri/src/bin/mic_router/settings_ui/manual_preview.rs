//! 説明書撮影専用。実デバイス・音声・ホットキー・常駐IPCへ接続しない。
use super::{Action, SettingsWindow};
use crate::config::{Mode, Settings};
use std::{path::Path, time::Duration};
use windows::Win32::UI::WindowsAndMessaging::*;
pub fn run(output: &Path) -> Result<(), String> {
    std::fs::create_dir(output).map_err(|e| e.to_string())?;
    let mut settings = Settings::defaults(output.to_path_buf());
    settings.microphone_id = "sample-mic".into(); settings.microphone_name = "サンプルマイク".into();
    settings.speaker_id = "sample-speaker".into(); settings.speaker_name = "サンプル再生".into();
    settings.cable_id = "sample-cable".into(); settings.cable_name = "サンプル配信先".into();
    settings.monitor_id = "sample-monitor".into(); settings.monitor_name = "サンプルモニター".into();
    let device = |id: &str, label: &str, direction: &str| serde_json::from_value(serde_json::json!({
        "endpoint_id": id, "friendly_name":label,"direction":direction,"state":"Active","is_default":false,
        "current_sample_rate":48000,"channels":2,"supported_sample_rates":[],"user_visible":true,"user_sort_order":0
    })).unwrap();
    let schema: audio_mixer_backend::plugins::package::PluginPackageManifest = serde_json::from_str(include_str!("../../../../../../plugins/rvc-cuda/plugin.json")).map_err(|e| e.to_string())?;
    let mut window = SettingsWindow::build(&settings, &[device("sample-mic","サンプルマイク","capture")], &[
        device("sample-speaker","サンプル再生","render"), device("sample-cable","サンプル配信先","render"), device("sample-monitor","サンプルモニター","render")], schema.descriptor.parameters, Mode::Muted, |_| {})?;
    window.show();
    loop {
        unsafe { let mut msg=MSG::default(); while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool() {
            if msg.message==WM_QUIT {return Ok(());}
            if !super::dialog_message(&msg) { let _=TranslateMessage(&msg);DispatchMessageW(&msg); }
        }}
        match window.poll() {
            Some(Action::Close) => return Ok(()),
            Some(Action::Save(next)) => {
                let json=serde_json::to_string_pretty(&next).map_err(|e|e.to_string())?;
                std::fs::write(output.join("settings.json"),json).map_err(|e|e.to_string())?;
                window.applied(&next); window.message("設定を保存しました。");
            },
            Some(Action::Preview(_, _)) => window.message("撮影用のため音は再生しません。"),
            None => {}
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}
