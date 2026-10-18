//! 音声イベントのピークを50msごとに集約。音声データ自体は保持しない。
use super::config::Mode;
use audio_mixer_backend::audio::{engine::RuntimeEvent, graph::NodeId};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const INTERVAL: Duration = Duration::from_millis(50);
pub const WINDOW: Duration = Duration::from_secs(1);
pub const STALE: Duration = Duration::from_millis(500);
pub const BINS: usize = 20;
pub type Shared = Arc<Mutex<Option<Snapshot>>>;

#[derive(Debug, Clone)]
pub struct Sample {
    pub at: Instant,
    pub input: Option<f32>,
    pub output: Option<f32>,
    pub outputs: [Option<f32>; 3],
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub generation: u64,
    pub mode: Mode,
    pub at: Instant,
    pub samples: Vec<Sample>,
    pub latency_ms: Option<f64>,
    pub outputs_enabled: [bool; 3],
}
impl Snapshot {
    pub fn fresh(&self, now: Instant, generation: u64, mode: Mode) -> bool {
        self.generation == generation
            && self.mode == mode
            && now.saturating_duration_since(self.at) <= STALE
    }
    /// 時間軸を固定し、観測の遅れを履歴の引き伸ばしにしない。
    pub fn bins(&self, now: Instant) -> [[Option<f32>; BINS]; 2] {
        let mut bins = [[None; BINS]; 2];
        for sample in &self.samples {
            let age = now.saturating_duration_since(sample.at);
            if age >= WINDOW {
                continue;
            }
            let index = BINS - 1 - (age.as_millis() / INTERVAL.as_millis()) as usize;
            for (row, value) in [sample.input, sample.output].into_iter().enumerate() {
                if let Some(value) = value {
                    accumulate(&mut bins[row][index], value);
                }
            }
        }
        bins
    }
    pub fn output_bins(&self, now: Instant) -> [[Option<f32>; BINS]; 3] {
        let mut bins = [[None; BINS]; 3];
        for sample in &self.samples {
            let age = now.saturating_duration_since(sample.at);
            if age >= WINDOW {
                continue;
            }
            let index = BINS - 1 - (age.as_millis() / INTERVAL.as_millis()) as usize;
            for (row, value) in sample.outputs.into_iter().enumerate() {
                if self.outputs_enabled[row] {
                    if let Some(value) = value {
                        accumulate(&mut bins[row][index], value);
                    }
                }
            }
        }
        bins
    }
}
pub struct Collector {
    pub input: NodeId,
    pub output: NodeId,
    pub input_enabled: bool,
    pub outputs_enabled: [bool; 3],
    output_nodes: [NodeId; 3],
    pending: [Option<f32>; 4],
    samples: VecDeque<Sample>,
    last: Instant,
    pub observed: u64,
}
fn accumulate(target: &mut Option<f32>, peak: f32) {
    if peak.is_finite() && peak >= 0.0 {
        *target = Some(target.unwrap_or(0.0).max(peak));
    }
}
impl Collector {
    pub fn new(input: NodeId, output_nodes: [NodeId; 3]) -> Self {
        Self {
            input,
            output: output_nodes[0],
            input_enabled: true,
            outputs_enabled: [true; 3],
            output_nodes,
            pending: [None; 4],
            samples: VecDeque::new(),
            last: Instant::now(),
            observed: 0,
        }
    }
    pub fn reset(&mut self) {
        self.pending = [None; 4];
        self.samples.clear();
        self.last = Instant::now();
    }
    pub fn observe(&mut self, event: &RuntimeEvent) {
        if let RuntimeEvent::NodeMeter { node_id, meter } = event {
            let row = if *node_id == self.input && self.input_enabled {
                0
            } else if let Some(index) = self
                .output_nodes
                .iter()
                .enumerate()
                .position(|(i, id)| id == node_id && self.outputs_enabled[i])
            {
                index + 1
            } else {
                return;
            };
            accumulate(&mut self.pending[row], meter.peak);
            self.observed += 1;
        }
    }
    pub fn publish(
        &mut self,
        now: Instant,
        generation: u64,
        mode: Mode,
        latency_ms: Option<f64>,
    ) -> Option<Snapshot> {
        if now.saturating_duration_since(self.last) < INTERVAL {
            return None;
        }
        self.last = now;
        self.samples
            .retain(|sample| now.saturating_duration_since(sample.at) < WINDOW);
        if self.samples.len() == BINS {
            self.samples.pop_front();
        }
        let outputs = std::array::from_fn(|i| {
            if self.outputs_enabled[i] {
                self.pending[i + 1].take()
            } else {
                self.pending[i + 1] = None;
                None
            }
        });
        self.samples.push_back(Sample {
            at: now,
            input: self.pending[0].take().filter(|_| self.input_enabled),
            output: self
                .output_nodes
                .iter()
                .position(|id| *id == self.output)
                .and_then(|i| outputs[i]),
            outputs,
        });
        Some(Snapshot {
            generation,
            mode,
            at: now,
            samples: self.samples.iter().cloned().collect(),
            latency_ms,
            outputs_enabled: self.outputs_enabled,
        })
    }
}
pub fn level_height(peak: f32) -> f32 {
    if !peak.is_finite() || peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_mixer_backend::audio::meter::MeterFrame;
    #[test]
    fn outputs_have_independent_peaks_and_disabled_paths_do_not_collect() {
        let input = NodeId::new();
        let outputs = [NodeId::new(), NodeId::new(), NodeId::new()];
        let mut c = Collector::new(input, outputs);
        c.outputs_enabled = [true, false, true];
        c.input_enabled = false;
        for (id, peak) in [
            (input, 1.0),
            (outputs[0], 0.1),
            (outputs[1], 0.9),
            (outputs[2], 0.7),
        ] {
            c.observe(&RuntimeEvent::NodeMeter {
                node_id: id,
                meter: MeterFrame {
                    peak,
                    ..Default::default()
                },
            });
        }
        let s = c.publish(c.last + INTERVAL, 9, Mode::Muted, None).unwrap();
        assert_eq!(c.observed, 2);
        assert_eq!(s.samples[0].input, None);
        assert_eq!(s.samples[0].outputs, [Some(0.1), None, Some(0.7)]);
        let bins = s.output_bins(s.at);
        assert_eq!(
            [bins[0][19], bins[1][19], bins[2][19]],
            [Some(0.1), None, Some(0.7)]
        );
        assert!(s
            .output_bins(s.at + WINDOW)
            .iter()
            .flatten()
            .all(Option::is_none));
    }
    #[test]
    fn peaks_are_routed_bounded_and_expire_without_stretching_time() {
        let input = NodeId::new();
        let output = NodeId::new();
        let monitor = NodeId::new();
        let mut c = Collector::new(input, [output, NodeId::new(), NodeId::new()]);
        let start = c.last;
        for (id, peak) in [
            (input, 0.2),
            (input, 0.7),
            (output, 0.3),
            (monitor, 1.0),
            (output, f32::NAN),
        ] {
            c.observe(&RuntimeEvent::NodeMeter {
                node_id: id,
                meter: MeterFrame {
                    peak,
                    ..Default::default()
                },
            });
        }
        let s = c
            .publish(start + INTERVAL, 1, Mode::Rvc, Some(350.0))
            .unwrap();
        assert_eq!(s.samples[0].input, Some(0.7));
        assert_eq!(s.samples[0].output, Some(0.3));
        assert_eq!(s.bins(s.at)[0][19], Some(0.7));
        assert!(s.fresh(s.at, 1, Mode::Rvc));
        assert!(!s.fresh(s.at + STALE + INTERVAL, 1, Mode::Rvc));
        assert!(!s.fresh(s.at, 2, Mode::Rvc));
        assert!(!s.fresh(s.at, 1, Mode::Direct));
        assert!(s.bins(s.at + WINDOW).iter().flatten().all(Option::is_none));
        for i in 2..=50 {
            c.publish(start + INTERVAL * i, 1, Mode::Rvc, None);
        }
        assert_eq!(c.samples.len(), BINS);
        assert!(c
            .samples
            .iter()
            .all(|s| s.input.is_none() && s.output.is_none()));
        c.reset();
        assert!(c.samples.is_empty());
        assert_eq!(level_height(0.0), 0.0);
        assert_eq!(level_height(1.0), 1.0);
        assert!((level_height(0.03162278) - 0.5).abs() < 0.001);
    }
}
