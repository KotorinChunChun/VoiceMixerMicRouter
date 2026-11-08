//! 仮想Endpointの解決と、製品の音声グラフを使ったメモリー内での送出試験。
use crate::{
    config::{Mode, Settings},
    engine,
};
use audio_mixer_backend::{
    audio::{
        engine::{AudioProcessor, AudioRuntime, AudioSink, AudioSource, RuntimeSettings},
        format::{AudioBlock, INTERNAL_SAMPLE_RATE},
        graph::{GraphSnapshot, NodeProperties},
    },
    windows::devices::{DEFAULT_COMMUNICATIONS_ENDPOINT, DEFAULT_CONSOLE_ENDPOINT},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Devices {
    capture: Endpoints,
    render: Endpoints,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoints {
    endpoints: HashSet<String>,
    console: Option<String>,
    communications: Option<String>,
}

impl Endpoints {
    fn resolve<'a>(&'a self, id: &'a str) -> Result<&'a str, String> {
        let resolved = match id {
            DEFAULT_CONSOLE_ENDPOINT => self.console.as_deref(),
            DEFAULT_COMMUNICATIONS_ENDPOINT => self.communications.as_deref(),
            _ => Some(id),
        }
        .ok_or_else(|| format!("仮想デバイスの既定先がありません: {id}"))?;
        if resolved.is_empty() || !self.endpoints.contains(resolved) {
            return Err(format!("仮想デバイスが未接続です: {resolved}"));
        }
        Ok(resolved)
    }
}

impl Devices {
    pub(super) fn validate_speaker_route(
        &self,
        settings: &Settings,
        mode: Mode,
    ) -> Result<(), String> {
        engine::validate_speaker_route_with(settings, mode, |id| {
            self.render.resolve(id).map(str::to_owned)
        })
    }

    pub(super) fn plan(&self, settings: &Settings, mode: Mode) -> Result<Plan, String> {
        self.validate_speaker_route(settings, mode)?;
        let input = match mode {
            Mode::Direct | Mode::Rvc => {
                Some(("microphone", self.capture.resolve(&settings.microphone_id)?))
            }
            Mode::SpeakerRvc => Some(("speaker", self.render.resolve(&settings.speaker_id)?)),
            Mode::Muted => None,
        };
        let mut outputs = [None, None, None];
        for (index, id) in [
            &settings.cable_id,
            &settings.voice_input_id,
            &settings.monitor_id,
        ]
        .into_iter()
        .enumerate()
        {
            if settings.output_flags()[index] {
                outputs[index] = Some(self.render.resolve(id)?.to_owned());
            }
        }
        let enabled = engine::enabled_outputs_with(settings, mode, |id| {
            self.render.resolve(id).map(str::to_owned)
        });
        let mut effective = settings.clone();
        [
            effective.cable_enabled,
            effective.voice_input_enabled,
            effective.monitoring,
        ] = enabled;
        let project = effective.project(mode)?;
        Ok(Plan {
            input: input
                .map(|(kind, endpoint_id)| json!({"kind": kind, "endpoint_id": endpoint_id})),
            simulated_rvc: mode.is_rvc(),
            outputs,
            enabled,
            graph: GraphSnapshot {
                nodes: project.nodes,
                edges: project.routing_edges,
                ..Default::default()
            },
        })
    }
}

pub(super) struct Plan {
    input: Option<Value>,
    simulated_rvc: bool,
    outputs: [Option<String>; 3],
    enabled: [bool; 3],
    graph: GraphSnapshot,
}

impl Plan {
    pub(super) fn probe(self) -> Result<Value, String> {
        let sinks: [_; 3] =
            std::array::from_fn(|_| Arc::new(Sink(AtomicU32::new(f32::NAN.to_bits()))));
        if self.enabled.iter().any(|on| *on) {
            // Settings::project は出力ノードへの接続を出力1～3の順で生成する。
            let sink_nodes = self
                .graph
                .edges
                .iter()
                .map(|edge| &self.graph.nodes[&edge.to_node])
                .filter(|node| matches!(node.properties, NodeProperties::Monitor { .. }));
            let graph = Arc::new(arc_swap::ArcSwap::from_pointee(self.graph.clone()));
            let runtime = AudioRuntime::new(graph, RuntimeSettings::default());
            for (node, sink) in sink_nodes.zip(&sinks) {
                runtime.bind_sink(node.id, sink.clone());
            }
            for node in self.graph.nodes.values() {
                match node.properties {
                    NodeProperties::Microphone { .. } => {
                        runtime.bind_source(node.id, Arc::new(Signal(0.25)))
                    }
                    NodeProperties::EndpointLoopback { .. } => {
                        runtime.bind_source(node.id, Arc::new(Signal(0.375)))
                    }
                    NodeProperties::TransformPlugin { .. } => {
                        runtime.bind_processor(node.id, Arc::new(SimulatedRvc))
                    }
                    _ => {}
                }
            }
            let handle = runtime.start().map_err(|error| error.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(2);
            while self
                .enabled
                .iter()
                .zip(&sinks)
                .any(|(on, sink)| *on && sink.peak().is_nan())
            {
                if Instant::now() >= deadline {
                    return Err("仮想出力で音声ブロックを受け取れませんでした".into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            handle.stop();
        }
        let outputs: Vec<_> = sinks
            .iter()
            .enumerate()
            .map(|(i, sink)| {
                let peak = sink.peak();
                json!({"output": i + 1, "endpoint_id": self.outputs[i], "active": self.enabled[i],
                "peak": peak.is_finite().then_some(peak)})
            })
            .collect();
        Ok(
            json!({"input": self.input, "simulated_rvc": self.simulated_rvc,
            "sample_rate_hz": INTERNAL_SAMPLE_RATE, "outputs": outputs}),
        )
    }
}

struct Signal(f32);
impl AudioSource for Signal {
    fn read_into(&self, block: &mut AudioBlock) -> audio_mixer_backend::Result<()> {
        block.samples.fill(self.0);
        Ok(())
    }
}

struct SimulatedRvc;
impl AudioProcessor for SimulatedRvc {
    fn process_in_place(&self, block: &mut AudioBlock) -> audio_mixer_backend::Result<()> {
        for sample in &mut block.samples {
            *sample *= 2.0;
        }
        Ok(())
    }
}

struct Sink(AtomicU32);
impl Sink {
    fn peak(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}
impl AudioSink for Sink {
    fn write(&self, block: &AudioBlock) -> audio_mixer_backend::Result<()> {
        let peak = block
            .samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0, f32::max);
        self.0.store(peak.to_bits(), Ordering::Relaxed);
        Ok(())
    }
}
