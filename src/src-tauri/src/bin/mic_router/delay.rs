//! 無音後の明瞭な立ち上がりだけを対応付ける、間欠的な遅延測定。
use audio_mixer_backend::audio::level_probe::{LevelObservation, LevelProbe};
use std::sync::Arc;

const MS: u64 = 10_000;
pub const QUIET_SECONDS: u64 = 3;
pub const UPDATE_SECONDS: u64 = 180;
const QUIET: u64 = QUIET_SECONDS * 1000 * MS;
const UPDATE: u64 = UPDATE_SECONDS * 1000 * MS;
const MAX_DELAY: u64 = 5_000 * MS;
const MIN_SOUND: u64 = 40 * MS;
const MAX_RISE: u64 = 300 * MS;
const MAX_GAP: u64 = 100 * MS;
// ヒステリシスを設け、部屋のノイズ床付近の揺れで測定しない。
const SILENCE_RMS: f32 = 0.003_162_278; // -50 dBFS
const SOUND_RMS: f32 = 0.01; // -40 dBFS

#[derive(Default)]
struct QuietRise {
    quiet_since: Option<u64>,
    transition_since: Option<u64>,
    sound_since: Option<u64>,
    last_end: Option<u64>,
}
impl QuietRise {
    fn observe(&mut self, sample: LevelObservation) -> Option<u64> {
        if !sample.rms.is_finite() || sample.duration_100ns == 0 {
            *self = Self::default();
            return None;
        }
        let at = sample.at_100ns;
        if self
            .last_end
            .is_some_and(|end| at > end.saturating_add(MAX_GAP) || at.saturating_add(30 * MS) < end)
        {
            *self = Self::default();
        }
        let end = at.saturating_add(sample.duration_100ns);
        self.last_end = Some(end);
        if sample.rms <= SILENCE_RMS {
            self.sound_since = None;
            self.transition_since = None;
            self.quiet_since.get_or_insert(at);
            return None;
        }
        if let Some(start) = self.sound_since {
            // 開始後は下側のしきい値まで継続扱いにする（音節の小さな谷を許容）。
            if end.saturating_sub(start) >= MIN_SOUND {
                self.sound_since = None;
                return Some(start);
            }
        } else if let Some(transition) = self.transition_since {
            if at.saturating_sub(transition) > MAX_RISE {
                self.transition_since = None;
                self.quiet_since = None;
            } else if sample.rms >= SOUND_RMS {
                self.transition_since = None;
                self.quiet_since = None;
                self.sound_since = Some(at);
            }
        } else if self
            .quiet_since
            .is_some_and(|quiet| at.saturating_sub(quiet) >= QUIET)
        {
            if sample.rms >= SOUND_RMS {
                self.quiet_since = None;
                self.sound_since = Some(at);
            } else {
                // 声は無音からしきい値まで数十msかけて立ち上がる。
                // 中間の音量で無音の履歴を失わず、短い遷移だけ許容する。
                self.transition_since = Some(at);
            }
        } else {
            self.quiet_since = None;
        }
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Measurement {
    pub milliseconds: f64,
    pub measured_at: u64,
}

#[derive(Default)]
pub struct Estimator {
    input: QuietRise,
    output: QuietRise,
    pending_input: Option<u64>,
    pub measurement: Option<Measurement>,
}
impl Estimator {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn should_observe(&self, now: u64) -> bool {
        self.measurement
            .is_none_or(|last| now.saturating_sub(last.measured_at) >= UPDATE)
    }
    pub fn status(&self, now: u64) -> &'static str {
        if !self.should_observe(now) {
            "cooldown"
        } else if self.pending_input.is_some() {
            "waiting_for_output"
        } else {
            "waiting_for_silence"
        }
    }
    pub fn tick(&mut self, now: u64) {
        if self
            .pending_input
            .is_some_and(|at| now.saturating_sub(at) > MAX_DELAY + MIN_SOUND)
        {
            self.pending_input = None;
        }
    }
    pub fn observe(&mut self, input: bool, sample: LevelObservation) {
        let now = sample.at_100ns.saturating_add(sample.duration_100ns);
        if !self.should_observe(now) {
            return;
        }
        self.tick(now);
        if input {
            if let Some(at) = self.input.observe(sample) {
                self.pending_input = Some(at);
            }
        } else if let Some(out) = self.output.observe(sample) {
            if let Some(at) = self.pending_input.take() {
                if let Some(delta) = out.checked_sub(at).filter(|d| *d <= MAX_DELAY) {
                    self.measurement = Some(Measurement {
                        milliseconds: delta as f64 / MS as f64,
                        measured_at: now,
                    });
                    self.input = QuietRise::default();
                    self.output = QuietRise::default();
                }
            }
        }
    }
}

struct Probes {
    input: Arc<LevelProbe>,
    output: Arc<LevelProbe>,
    input_revision: (u64, u64),
    output_revision: (u64, u64),
}
impl Drop for Probes {
    fn drop(&mut self) {
        self.input.set_enabled(false);
        self.output.set_enabled(false);
    }
}

#[derive(Default)]
pub struct Meter {
    probes: Option<Probes>,
    estimator: Estimator,
}
impl Meter {
    pub fn reset(&mut self) {
        self.probes = None;
        self.estimator.reset();
    }
    pub fn update(
        &mut self,
        input: Option<Arc<LevelProbe>>,
        output: Option<Arc<LevelProbe>>,
        now: u64,
    ) -> Option<f64> {
        let (Some(input), Some(output)) = (input, output) else {
            self.reset();
            return None;
        };
        let input_revision = input.revision();
        let output_revision = output.revision();
        let changed = self.probes.as_ref().is_none_or(|p| {
            !Arc::ptr_eq(&p.input, &input)
                || !Arc::ptr_eq(&p.output, &output)
                || p.input_revision != input_revision
                || p.output_revision != output_revision
        });
        if changed {
            self.reset();
            self.probes = Some(Probes {
                input,
                output,
                input_revision,
                output_revision,
            });
        }
        let probes = self.probes.as_ref().unwrap();
        let observe = self.estimator.should_observe(now);
        probes.input.set_enabled(observe);
        probes.output.set_enabled(observe);
        if observe {
            let mut samples = Vec::new();
            while let Some(sample) = probes.input.pop() {
                samples.push((true, sample));
            }
            while let Some(sample) = probes.output.pop() {
                samples.push((false, sample));
            }
            // 受信した順序ではなく取得・再生の時刻で対応付ける。
            samples.sort_by_key(|(_, sample)| sample.at_100ns);
            if samples.iter().any(|(_, sample)| {
                now.saturating_sub(sample.at_100ns) > 1000 * MS
                    || sample.at_100ns > now.saturating_add(500 * MS)
            }) {
                self.estimator.reset();
                return None;
            }
            for (is_input, sample) in samples {
                self.estimator.observe(is_input, sample);
            }
            self.estimator.tick(now);
            if !self.estimator.should_observe(now) {
                probes.input.set_enabled(false);
                probes.output.set_enabled(false);
            }
        }
        self.estimator.measurement.map(|m| m.milliseconds)
    }
    pub fn diagnostics(&self, now: u64) -> serde_json::Value {
        serde_json::json!({
            "method":"quiet_onset",
            "state":if self.probes.is_some() { self.estimator.status(now) } else { "inactive" },
            "milliseconds":self.estimator.measurement.map(|m| m.milliseconds),
            "age_seconds":self.estimator.measurement.map(|m| now.saturating_sub(m.measured_at) as f64 / 10_000_000.0),
            "quiet_seconds":QUIET_SECONDS,
            "minimum_update_seconds":UPDATE_SECONDS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        e: &mut Estimator,
        start_ms: u64,
        duration_ms: u64,
        input: impl Fn(u64) -> f32,
        output: impl Fn(u64) -> f32,
    ) {
        for elapsed in (0..duration_ms).step_by(10) {
            for (is_input, rms) in [(true, input(elapsed)), (false, output(elapsed))] {
                e.observe(
                    is_input,
                    LevelObservation {
                        at_100ns: (start_ms + elapsed) * MS,
                        duration_100ns: 10 * MS,
                        rms,
                    },
                );
            }
        }
    }
    fn pulse(t: u64, onset: u64, width: u64, amplitude: f32) -> f32 {
        if (onset..onset + width).contains(&t) {
            amplitude
        } else {
            0.0001
        }
    }
    #[test]
    fn onset_measures_real_delay_even_if_rvc_changes_volume() {
        let mut e = Estimator::default();
        run(
            &mut e,
            0,
            6000,
            |t| pulse(t, 3200, 250, 0.4),
            |t| pulse(t, 4470, 250, 0.04),
        );
        assert_eq!(e.measurement.unwrap().milliseconds, 1270.0);
        assert_eq!(e.status(6000 * MS), "cooldown");
    }
    #[test]
    fn a_short_voice_ramp_can_cross_the_hysteresis_band() {
        let mut e = Estimator::default();
        let voice = |t, onset| {
            if (onset - 70..onset).contains(&t) {
                0.006
            } else {
                pulse(t, onset, 250, 0.05)
            }
        };
        run(&mut e, 0, 6000, |t| voice(t, 3200), |t| voice(t, 4470));
        assert_eq!(e.measurement.unwrap().milliseconds, 1270.0);
        let mut slight_noise = Estimator::default();
        run(
            &mut slight_noise,
            0,
            6000,
            |t| {
                if (3100..3120).contains(&t) {
                    0.006
                } else {
                    pulse(t, 3500, 250, 0.05)
                }
            },
            |t| pulse(t, 4770, 250, 0.05),
        );
        assert_eq!(slight_noise.measurement.unwrap().milliseconds, 1270.0);
        let mut syllable = Estimator::default();
        let voice = |t, onset| {
            if (onset + 30..onset + 50).contains(&t) {
                0.006
            } else {
                pulse(t, onset, 250, 0.05)
            }
        };
        run(
            &mut syllable,
            0,
            6000,
            |t| voice(t, 3200),
            |t| voice(t, 4470),
        );
        assert_eq!(syllable.measurement.unwrap().milliseconds, 1270.0);
        let mut slow = Estimator::default();
        run(
            &mut slow,
            0,
            6000,
            |t| {
                if (3100..4000).contains(&t) {
                    0.006
                } else {
                    pulse(t, 4000, 250, 0.05)
                }
            },
            |t| pulse(t, 4470, 250, 0.05),
        );
        assert!(slow.measurement.is_none());
    }
    #[test]
    fn silence_clicks_continuous_audio_and_missing_output_stay_unknown() {
        for case in 0..4 {
            let mut e = Estimator::default();
            run(
                &mut e,
                0,
                10_000,
                |t| match case {
                    0 => 0.0,
                    1 => pulse(t, 3200, 10, 1.0),
                    2 => 0.4,
                    _ => pulse(t, 3200, 250, 0.4),
                },
                |t| match case {
                    1 => pulse(t, 4470, 10, 1.0),
                    2 => 0.4,
                    _ => 0.0,
                },
            );
            e.tick(10_000 * MS);
            assert!(e.measurement.is_none(), "case {case}");
            assert!(e.pending_input.is_none());
        }
    }
    #[test]
    fn measurements_hold_for_three_minutes_and_restart_with_new_silence() {
        let mut e = Estimator::default();
        run(
            &mut e,
            0,
            6000,
            |t| pulse(t, 3200, 250, 0.2),
            |t| pulse(t, 4470, 250, 0.2),
        );
        run(
            &mut e,
            30_000,
            6000,
            |t| pulse(t, 3200, 250, 0.2),
            |t| pulse(t, 3500, 250, 0.2),
        );
        assert_eq!(e.measurement.unwrap().milliseconds, 1270.0);
        assert!(!e.should_observe(184_509 * MS));
        run(
            &mut e,
            185_000,
            6000,
            |t| pulse(t, 3200, 250, 0.2),
            |t| pulse(t, 3900, 250, 0.2),
        );
        assert_eq!(e.measurement.unwrap().milliseconds, 700.0);
        e.reset();
        assert!(e.measurement.is_none());
    }
    #[test]
    fn a_suppressed_click_does_not_replace_the_following_voice_measurement() {
        let mut e = Estimator::default();
        run(
            &mut e,
            0,
            10_000,
            |t| pulse(t, 3200, 20, 0.5) + pulse(t, 7000, 250, 0.2),
            |t| pulse(t, 8070, 250, 0.2),
        );
        assert_eq!(e.measurement.unwrap().milliseconds, 1070.0);
        e.reset();
        assert!(e.measurement.is_none());
    }
    #[test]
    fn a_gap_is_not_counted_as_silence_and_late_or_early_output_is_rejected() {
        let mut e = Estimator::default();
        run(&mut e, 0, 100, |_| 0.0, |_| 0.0);
        run(
            &mut e,
            4000,
            2000,
            |t| pulse(t, 0, 250, 0.4),
            |t| pulse(t, 1270, 250, 0.4),
        );
        assert!(e.measurement.is_none());
        for offset in [3100, 9000] {
            let mut e = Estimator::default();
            run(
                &mut e,
                0,
                11_000,
                |t| pulse(t, 3200, 250, 0.4),
                |t| pulse(t, offset, 250, 0.4),
            );
            assert!(e.measurement.is_none());
        }
    }

    #[test]
    fn device_probes_hold_values_but_reconnection_and_target_change_invalidate_them() {
        let input = Arc::new(LevelProbe::default());
        let output = Arc::new(LevelProbe::default());
        let mut meter = Meter::default();
        let mut value = None;
        for t in (0..6000).step_by(10) {
            input.observe(&vec![pulse(t, 3200, 250, 0.4); 960], t * MS, 48_000, 2);
            output.observe(&vec![pulse(t, 4470, 250, 0.04); 960], t * MS, 48_000, 2);
            value = meter.update(Some(input.clone()), Some(output.clone()), (t + 10) * MS);
        }
        assert_eq!(value, Some(1270.0));
        assert_eq!(
            meter.update(Some(input.clone()), Some(output.clone()), 170_000 * MS),
            value
        );
        // 3分間は音量観測を止めても前回値を表示する。
        input.observe(&[0.4; 960], 171_000 * MS, 48_000, 2);
        assert!(input.pop().is_none());
        output.restart();
        assert_eq!(
            meter.update(Some(input.clone()), Some(output.clone()), 172_000 * MS),
            None
        );
        assert!(meter.estimator.measurement.is_none());
        meter.estimator.measurement = Some(Measurement {
            milliseconds: 900.0,
            measured_at: 172_000 * MS,
        });
        let another = Arc::new(LevelProbe::default());
        assert_eq!(
            meter.update(Some(input.clone()), Some(another), 172_100 * MS),
            None
        );
        meter.update(None, None, 172_200 * MS);
        input.observe(&[0.4; 960], 172_200 * MS, 48_000, 2);
        assert!(input.pop().is_none());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "追加の仮想音声デバイスを使う。VOICEMIXER_DELAY_SOURCE／INPUT／OUTPUTにEndpoint IDを指定する"]
    fn virtual_device_delay_covers_queued_audio() {
        use audio_mixer_backend::{
            audio::{
                clock::now_100ns,
                graph::{Edge, NodeProperties, NodeSpec, RecordingFormat},
            },
            config::{model::PersistedConfiguration, persistence::ConfigStore},
            recording::codecs::encode_audio_file,
            standalone::StandaloneSession,
        };
        use std::{
            thread,
            time::{Duration, Instant},
        };

        let endpoint = |key| std::env::var(key).expect("試験専用の仮想デバイスを指定してください");
        let source_endpoint = endpoint("VOICEMIXER_DELAY_SOURCE");
        let input_endpoint = endpoint("VOICEMIXER_DELAY_INPUT");
        let output_endpoint = endpoint("VOICEMIXER_DELAY_OUTPUT");
        assert_ne!(source_endpoint, output_endpoint, "入力への音響帰還を避ける");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tmp")
            .join(format!("mic-delay-known-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let audio = (0..48_000 * 12)
            .flat_map(|frame| {
                let value = if (48_000 * 6..48_000 * 7).contains(&frame) {
                    0.2 * (std::f32::consts::TAU * 440.0 * frame as f32 / 48_000.0).sin()
                } else {
                    0.0
                };
                [value, value]
            })
            .collect::<Vec<_>>();
        encode_audio_file(root.join("signal.wav"), RecordingFormat::Wav, &audio).unwrap();
        let node = |name: &str, properties: serde_json::Value| {
            let properties: NodeProperties = serde_json::from_value(properties).unwrap();
            NodeSpec::new(properties.node_type(), name, properties)
        };
        let mut results = Vec::new();
        for delay_ms in [0.0, 1250.0] {
            let source = node(
                "試験音",
                serde_json::json!({"kind":"file_input","path":"signal.wav","looped":false,"gain_db":0.0,"mute":false}),
            );
            let source_output = node(
                "入力用仮想再生",
                serde_json::json!({"kind":"monitor","endpoint_id":source_endpoint,"gain_db":0.0,"mute":false}),
            );
            let input = node(
                "測定入力",
                serde_json::json!({"kind":"microphone","endpoint_id":input_endpoint,"gain_db":0.0,"mute":false}),
            );
            let delay = node(
                "既知の遅延",
                serde_json::json!({"kind":"delay","config":{"delay_ms":delay_ms},"gain_db":0.0,"mute":false}),
            );
            let output = node(
                "測定出力",
                serde_json::json!({"kind":"monitor","endpoint_id":output_endpoint,"gain_db":0.0,"mute":false}),
            );
            let mut config = PersistedConfiguration::default();
            config.settings.render_exclusive = false;
            for (from, to) in [
                (&source, &source_output),
                (&input, &delay),
                (&delay, &output),
            ] {
                config.routing_edges.push(Edge::new(
                    from.id,
                    from.output_ports[0].id,
                    to.id,
                    to.input_ports[0].id,
                ));
            }
            for value in [&source, &source_output, &input, &delay, &output] {
                config.nodes.insert(value.id, value.clone());
            }
            let path = root.join("project.json");
            ConfigStore::new(&path).save(&config).unwrap();
            let mut session = StandaloneSession::load(&path, None).unwrap();
            session.start().unwrap();
            let mut meter = Meter::default();
            let start = Instant::now();
            let value = loop {
                session.tick().unwrap();
                let backend = session.backend();
                let value = meter.update(
                    backend.nodes.level_probe(input.id),
                    backend.nodes.level_probe(output.id),
                    now_100ns(),
                );
                if start.elapsed() < Duration::from_secs(3) {
                    assert!(value.is_none());
                }
                if let Some(value) = value {
                    break value;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(12),
                    "測定待ちのまま: {}",
                    meter.diagnostics(now_100ns())
                );
                thread::sleep(Duration::from_millis(10));
            };
            results.push(serde_json::json!({"inserted_ms":delay_ms,"measured_ms":value,"diagnostics":meter.diagnostics(now_100ns())}));
            meter.reset();
            session.stop().unwrap();
        }
        std::fs::write(
            root.join("result.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
        let baseline = results[0]["measured_ms"].as_f64().unwrap();
        let delayed = results[1]["measured_ms"].as_f64().unwrap();
        println!(
            "結果 {}: 基準 {baseline:.1}ms、1250ms追加 {delayed:.1}ms",
            root.display()
        );
        assert!(baseline < 300.0, "仮想経路の基準値が大きすぎる");
        assert!(
            (delayed - baseline - 1250.0).abs() < 120.0,
            "キュー通過後の差を測れていない"
        );
    }
}
