//! 常駐音声制御の公開境界。経路検査とセッション内部の状態管理は子モジュールへ置く。

#[path = "engine/routing.rs"]
mod routing;
#[path = "engine/session.rs"]
mod session;

pub use routing::select_graph;
#[cfg(windows)]
pub(crate) use routing::{enabled_outputs_with, validate_speaker_route_with};
pub use session::spawn;

#[cfg(test)]
mod codec_tests {
    use audio_mixer_backend::{
        audio::{buffer::AudioRingBuffer, graph::RecordingFormat},
        recording::{
            codecs::{decode_audio_file, encode_audio_file},
            writer::RecorderSink,
        },
    };

    #[test]
    fn flac_batch_and_streaming_roundtrip_preserve_samples_and_finalize_on_stop() {
        let directory = tempfile::tempdir().unwrap();
        for frames in [1, 15, 4096, 8193] {
            let samples: Vec<f32> = (0..frames * 2)
                .map(|index| ((index % 1024) as f32 - 512.0) / 1024.0)
                .collect();
            let batch = directory.path().join("batch.flac");
            encode_audio_file(&batch, RecordingFormat::Flac, &samples).unwrap();
            let ring = AudioRingBuffer::new(samples.len() + 2);
            assert_eq!(ring.push_slice(&samples), samples.len());
            let stream = directory.path().join("stream.flac");
            let sink = RecorderSink::start_with_ring(&stream, RecordingFormat::Flac, ring).unwrap();
            sink.stop().unwrap();
            sink.stop().unwrap();
            for path in [&batch, &stream] {
                let decoded = decode_audio_file(path).unwrap();
                assert_eq!(decoded.samples.len(), samples.len(), "{frames} frames");
                for (actual, expected) in decoded.samples.iter().zip(&samples) {
                    assert!((actual - expected).abs() < 0.000001, "{frames} frames");
                }
            }
        }
    }
}

use super::config::{write_json, Mode, Settings};
use audio_mixer_backend::{audio::graph::NodeProperties, standalone::StandaloneSession};
use routing::{enabled_outputs, validate_speaker_route};
use serde::Serialize;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc, Arc,
    },
};

pub enum Command {
    Hold([bool; 3]),
    Sound(u8, String),
    Set(Mode, u64),
    Configure(Box<Settings>, u64),
    ReleaseRvc(u64),
    Restart(u64),
    Exit,
}
pub struct CommandSender {
    tx: mpsc::Sender<Command>,
    route_request: Arc<AtomicU8>,
}
impl CommandSender {
    pub fn send(&self, command: Command) -> Result<(), mpsc::SendError<Command>> {
        match command {
            Command::Set(mode, _) => self.route_request.store(
                match mode {
                    Mode::Direct => 1,
                    Mode::Muted => 2,
                    _ => 0,
                },
                Ordering::SeqCst,
            ),
            Command::ReleaseRvc(_) => self.route_request.store(1, Ordering::SeqCst),
            Command::Exit => self.route_request.store(2, Ordering::SeqCst),
            _ => {}
        }
        self.tx.send(command)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub mode: Mode,
    pub generation: u64,
    pub phase: String,
    pub message: String,
    pub diagnostics: Option<serde_json::Value>,
    pub applied: Option<bool>,
}
impl Status {
    pub fn new(mode: Mode, generation: u64, phase: &str, message: String) -> Self {
        Self {
            mode,
            generation,
            phase: phase.into(),
            message,
            diagnostics: None,
            applied: None,
        }
    }
}
pub fn start(
    settings: &Settings,
    directory: &Path,
    mode: Mode,
) -> Result<StandaloneSession, String> {
    let mode = settings.available_mode(mode);
    let path = directory.join("resident-project.json");
    let mut normalized = settings.clone();
    normalized.rvc_parameters = complete_parameters(settings)?;
    validate_speaker_route(settings, mode)?;
    let outputs = enabled_outputs(settings, mode);
    normalized.cable_enabled = outputs[0];
    normalized.voice_input_enabled = outputs[1];
    normalized.monitoring = outputs[2];
    write_json(&path, &normalized.project(mode)?)?;
    let mut session = StandaloneSession::load(&path, None).map_err(|e| e.to_string())?;
    session.start().map_err(|e| e.to_string())?;
    Ok(session)
}
fn complete_parameters(settings: &Settings) -> Result<serde_json::Value, String> {
    let package = audio_mixer_backend::plugins::package::load_package_manifest(
        settings.plugin_directory.join("rvc-cuda/plugin.json"),
    )
    .map_err(|e| e.to_string())?;
    let mut parameters = serde_json::Map::new();
    for p in package.descriptor.parameters {
        parameters.insert(p.id, p.default);
    }
    parameters.extend(
        settings
            .rvc_parameters
            .as_object()
            .ok_or("RVC設定が不正です")?
            .clone(),
    );
    Ok(serde_json::Value::Object(parameters))
}
pub fn diagnostics(session: &StandaloneSession) -> serde_json::Value {
    let mut report = serde_json::to_value(session.diagnostics()).unwrap_or_default();
    let plugins: Vec<_> = session
        .backend()
        .controller
        .list_nodes()
        .into_iter()
        .filter_map(|node| {
            let NodeProperties::TransformPlugin { instance_id, .. } = node.properties else {
                return None;
            };
            Some(serde_json::json!({
                "node": node.display_name,
                "instance": instance_id,
                "loaded": session.backend().nodes.is_active(node.id),
                "stats": audio_mixer_backend::commands::tauri_commands::get_plugin_stats(
                    session.backend(), node.id
                ).ok(),
            }))
        })
        .collect();
    report["plugins"] = serde_json::json!(plugins);
    report["latency"] = serde_json::json!(session.backend().runtime.latency_report());
    report
}
