//! モード選択とOS既定デバイスの解決、循環・重複出力の検査。

use super::super::config::{Mode, Settings, OUTPUT_SELECTOR, SOURCE_SELECTOR};
use audio_mixer_backend::audio::graph::{GraphSnapshot, NodeProperties};

/// ポートとRVCへの入力を同じGraph世代で変更する。原音と変換音を混ぜない。
pub fn select_graph(graph: &mut GraphSnapshot, mode: Mode) -> Result<(), String> {
    let rvc = graph
        .nodes
        .values()
        .find(|n| matches!(n.properties, NodeProperties::TransformPlugin { .. }))
        .ok_or("RVCノードがありません")?
        .id;
    for (name, muted, port) in [
        (
            OUTPUT_SELECTOR,
            mode == Mode::Muted,
            usize::from(mode.is_rvc()),
        ),
        (
            SOURCE_SELECTOR,
            !mode.is_rvc(),
            usize::from(mode == Mode::SpeakerRvc),
        ),
    ] {
        let selector = graph
            .nodes
            .values_mut()
            .find(|n| {
                n.display_name == name
                    && matches!(n.properties, NodeProperties::InputSelector { .. })
            })
            .ok_or("切替ノードがありません")?;
        if let NodeProperties::InputSelector { config, mute, .. } = &mut selector.properties {
            *mute = muted;
            config.active_port_id = Some(selector.input_ports[port].id);
            config.selection_revision = config.selection_revision.wrapping_add(1);
        }
    }
    for edge in &mut graph.edges {
        if edge.to_node == rvc {
            edge.mute = !mode.is_rvc();
        }
    }
    Ok(())
}
fn resolve_render(id: &str) -> Result<String, String> {
    audio_mixer_backend::windows::devices::resolve_endpoint_id(
        id,
        audio_mixer_backend::windows::devices::DeviceDirection::Render,
    )
    .map_err(|e| e.to_string())
}
pub(super) fn validate_speaker_route(settings: &Settings, mode: Mode) -> Result<(), String> {
    validate_speaker_route_with(settings, mode, resolve_render)
}
pub(super) fn validate_speaker_route_with(
    settings: &Settings,
    mode: Mode,
    mut resolve: impl FnMut(&str) -> Result<String, String>,
) -> Result<(), String> {
    if mode == Mode::SpeakerRvc {
        let speaker = resolve(&settings.speaker_id)?;
        for (index, (enabled, id)) in [
            (settings.cable_enabled, &settings.cable_id),
            (settings.voice_input_enabled, &settings.voice_input_id),
            (settings.monitoring, &settings.monitor_id),
        ]
        .into_iter()
        .enumerate()
        {
            if enabled && speaker == resolve(id)? {
                return Err(format!("SPK-RVCの入力スピーカーと{}が同じです。出力をOFFにするか、異なるデバイスを選択してください（音声の循環を防ぐため送信停止）", settings.output_label(index)));
            }
        }
    }
    Ok(())
}
pub(super) fn enabled_outputs(settings: &Settings, mode: Mode) -> [bool; 3] {
    enabled_outputs_with(settings, mode, resolve_render)
}
pub(super) fn enabled_outputs_with(
    settings: &Settings,
    mode: Mode,
    mut resolve: impl FnMut(&str) -> Result<String, String>,
) -> [bool; 3] {
    let mut resolved = Vec::new();
    let mut result = [false; 3];
    for (index, (enabled, id)) in [
        (settings.cable_enabled, &settings.cable_id),
        (settings.voice_input_enabled, &settings.voice_input_id),
        (settings.monitoring, &settings.monitor_id),
    ]
    .into_iter()
    .enumerate()
    {
        if !enabled {
            continue;
        }
        result[index] = match resolve(id) {
            Ok(endpoint) => {
                let unique = !resolved.contains(&endpoint);
                resolved.push(endpoint.clone());
                unique
                    && (mode != Mode::SpeakerRvc
                        || resolve(&settings.speaker_id).is_ok_and(|speaker| speaker != endpoint))
            }
            Err(_) => true, // ONの未接続先は通常の起動エラーとして扱う。
        };
    }
    result
}
#[cfg(test)]
pub(super) fn monitor_enabled_with(
    settings: &Settings,
    mode: Mode,
    mut resolve: impl FnMut(&str) -> Result<String, String>,
) -> bool {
    settings.monitoring && enabled_outputs_with(settings, mode, &mut resolve)[2]
}
