//! デバイス候補と初回導入時の選択。
use audio_mixer_backend::windows::devices::{
    AudioDeviceInfo, DEFAULT_COMMUNICATIONS_ENDPOINT, DEFAULT_CONSOLE_ENDPOINT,
};

pub fn is_default_device(id: &str) -> bool {
    matches!(
        id,
        DEFAULT_CONSOLE_ENDPOINT | DEFAULT_COMMUNICATIONS_ENDPOINT
    )
}
pub fn device_choices(devices: &[AudioDeviceInfo], id: &str, name: &str) -> Vec<(String, String)> {
    let mut choices = vec![
        (DEFAULT_CONSOLE_ENDPOINT.into(), "既定のデバイス".into()),
        (
            DEFAULT_COMMUNICATIONS_ENDPOINT.into(),
            "既定の通信デバイス".into(),
        ),
    ];
    choices.extend(
        devices
            .iter()
            .filter(|d| d.state == "Active")
            .map(|d| (d.endpoint_id.clone(), d.friendly_name.clone())),
    );
    if !id.is_empty() && !choices.iter().any(|(key, _)| key == id) {
        choices.push((id.into(), format!("{name}（未接続）")));
    }
    // 既定2項目は先頭に固定。同名は元の順を保ち、デバイスIDとの対応を崩さない。
    choices[2..].sort_by_cached_key(|(_, name)| name.to_lowercase());
    choices
}

pub(super) fn choose_voice_input(devices: &[AudioDeviceInfo]) -> Option<&AudioDeviceInfo> {
    let mut found = devices.iter().filter(|d| {
        d.state == "Active" && d.friendly_name.to_ascii_uppercase().starts_with("CABLE-D")
    });
    let candidate = found.next()?;
    if found.next().is_some() {
        None
    } else {
        Some(candidate)
    }
}
pub fn choose_microphone(devices: &[AudioDeviceInfo]) -> Result<&AudioDeviceInfo, String> {
    let device = devices
        .iter()
        .find(|d| d.is_default && d.state == "Active")
        .ok_or("既定マイクがありません")?;
    let name = device.friendly_name.to_ascii_lowercase();
    if name.contains("cable") || name.contains("voicemeeter") || name.contains("virtual") {
        return Err(
            "既定入力が仮想デバイスです。ループを避けるため実マイクのIDを指定してください".into(),
        );
    }
    Ok(device)
}

pub fn choose_cable(devices: &[AudioDeviceInfo]) -> Result<&AudioDeviceInfo, String> {
    // CABLE-C/D や 16ch をあいまいな部分一致で選ばない。
    let candidates: Vec<_> = devices
        .iter()
        .filter(|d| {
            d.state == "Active"
                && matches!(
                    d.friendly_name.as_str(),
                    "CABLE (VB-Audio Virtual Cable)" | "CABLE Input (VB-Audio Virtual Cable)"
                )
        })
        .collect();
    if candidates.len() != 1 {
        return Err("通常のVB-CABLE出力を一意に選べません。cable_idを明示してください".into());
    }
    Ok(candidates[0])
}
