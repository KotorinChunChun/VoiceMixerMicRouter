//! 背景準備・障害分離・設定復元と実機経路の回帰試験。

use super::super::routing::{
    enabled_outputs_with, monitor_enabled_with, validate_speaker_route_with,
};
use super::super::{Command, CommandSender};
use crate::config::{write_json, OUTPUT_SELECTOR};
use std::sync::mpsc;

use super::*;
use audio_mixer_backend::windows::devices::{list_devices, DeviceDirection};

fn wait_for_rvc(active: &mut ResidentSession) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while active.preparation.pending() {
        assert!(Instant::now() < deadline, "RVC準備の完了待ちが時間切れです");
        active.tick_audio().unwrap();
        active.poll_rvc_prepare().unwrap();
        thread::sleep(Duration::from_millis(10));
    }
}

fn isolated_settings(directory: &Path) -> Settings {
    serde_json::from_value(serde_json::json!({
        "microphone_id":"mic", "microphone_name":"Mic",
        "speaker_id":"speaker", "speaker_name":"Speaker",
        "cable_id":"output", "cable_name":"Output", "cable_enabled":false,
        "voice_input_id":"voice", "voice_input_name":"Voice", "voice_input_enabled":false,
        "monitor_id":"monitor", "monitor_name":"Monitor", "monitoring":false,
        "plugin_directory":directory, "rvc_parameters":{}, "last_mode":"direct"
    }))
    .unwrap()
}

// WASAPIも音声クロックも起動せず、本物のBackendと経路変更を使う。
fn isolated_session(directory: &Path) -> ResidentSession {
    let settings = isolated_settings(directory);
    let mut project = settings.project(Mode::Muted).unwrap();
    for node in project.nodes.values_mut() {
        node.runtime_enabled = false;
        if matches!(
            node.properties,
            NodeProperties::Microphone { .. } | NodeProperties::EndpointLoopback { .. }
        ) {
            node.properties = NodeProperties::TestSignal {
                config: Default::default(),
                gain_db: 0.0,
                mute: false,
            };
            node.node_type = node.properties.node_type();
        }
    }
    let id = |name: &str| {
        project
            .nodes
            .values()
            .find(|node| node.display_name == name)
            .unwrap()
            .id
    };
    let microphone = id("Mic");
    let speaker = id("Speaker");
    let output = id("Output");
    let voice_input = id(VOICE_INPUT_OUTPUT);
    let monitor = id("音声モニタリング");
    let rvc = id("RVC");
    let path = directory.join("isolated.json");
    write_json(&path, &project).unwrap();
    let session = StandaloneSession::load(&path, None).unwrap();
    let descriptor = session
        .backend()
        .plugins
        .list_plugins()
        .into_iter()
        .find(|d| d.input_ports.len() == 1 && d.output_ports.len() == 1)
        .unwrap();
    let mut node = session.backend().controller.snapshot().nodes[&rvc].clone();
    if let NodeProperties::TransformPlugin {
        plugin_id,
        parameters,
        ..
    } = &mut node.properties
    {
        *plugin_id = descriptor.plugin_id.clone();
        *parameters = descriptor.default_parameters();
    }
    session.backend().controller.update_node(node).unwrap();
    ResidentSession {
        output_holds: [false; 3],
        session,
        settings,
        mode: Mode::Muted,
        rvc,
        rvc_starts: 0,
        last_monitor_check: Instant::now(),
        meters: Collector::new(microphone, [output, voice_input, monitor]),
        delay: delay::Meter::default(),
        delay_target: "none",
        monitor,
        output,
        voice_input,
        microphone,
        speaker,
        route_request: Arc::new(AtomicU8::new(0)),
        preparation: Preparation::default(),
        release_after_prepare: false,
        release_only: false,
        rvc_wanted: false,
        rvc_retry: None,
        rvc_recoveries: 0,
        rvc_error: None,
        configuration: None,
        configuration_response: None,
    }
}

fn gated_preparation(active: &mut ResidentSession, fail: bool) -> mpsc::Sender<()> {
    let backend = active.session.backend().clone();
    let NodeProperties::TransformPlugin {
        instance_id,
        plugin_id,
        parameters,
        ..
    } = backend.controller.snapshot().nodes[&active.rvc]
        .properties
        .clone()
    else {
        unreachable!()
    };
    let (entered, started) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    active.preparation.begin(move || {
        entered.send(()).unwrap();
        wait.recv().map_err(|e| e.to_string())?;
        if fail {
            return Err("準備失敗の試験".into());
        }
        backend
            .plugins
            .create_instance(
                instance_id,
                &plugin_id,
                parameters,
                PluginPrepareContext {
                    sample_rate: 48000,
                    channels: 2,
                    graph_block_frames: 480,
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    });
    started.recv_timeout(Duration::from_secs(3)).unwrap();
    release
}

struct ModelStep {
    model: String,
    release: mpsc::Receiver<()>,
    fail: bool,
}
struct ModelGate {
    steps: std::sync::Mutex<std::collections::VecDeque<ModelStep>>,
    entered: mpsc::Sender<String>,
}
struct ModelFactory {
    descriptor: audio_mixer_backend::plugins::manifest::PluginDescriptor,
    gate: Arc<ModelGate>,
}
struct TestModel {
    model: String,
    gate: Arc<ModelGate>,
}
impl audio_mixer_backend::plugins::manager::PluginFactory for ModelFactory {
    fn descriptor(&self) -> audio_mixer_backend::plugins::manifest::PluginDescriptor {
        self.descriptor.clone()
    }
    fn create(
        &self,
        parameters: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> audio_mixer_backend::Result<
        Box<dyn audio_mixer_backend::plugins::instance::TransformInstance>,
    > {
        Ok(Box::new(TestModel {
            model: parameters["model"].as_str().unwrap().into(),
            gate: self.gate.clone(),
        }))
    }
}
impl audio_mixer_backend::plugins::instance::TransformInstance for TestModel {
    fn prepare(&mut self, _: PluginPrepareContext) -> audio_mixer_backend::Result<()> {
        let step = {
            let mut steps = self.gate.steps.lock().unwrap();
            if steps.front().is_some_and(|step| step.model == self.model) {
                steps.pop_front()
            } else {
                None
            }
        };
        if let Some(step) = step {
            self.gate.entered.send(self.model.clone()).unwrap();
            step.release.recv_timeout(Duration::from_secs(5)).unwrap();
            if step.fail {
                return Err(audio_mixer_backend::BackendError::Plugin(format!(
                    "{} のモデル読込失敗",
                    self.model
                )));
            }
        }
        Ok(())
    }
    fn start(&mut self) -> audio_mixer_backend::Result<()> {
        if self.model == "start-fail" {
            Err(audio_mixer_backend::BackendError::Plugin(
                "新モデルの開始失敗".into(),
            ))
        } else {
            Ok(())
        }
    }
    fn process(
        &mut self,
        _: &mut audio_mixer_backend::audio::format::AudioBlock,
    ) -> audio_mixer_backend::Result<()> {
        Ok(())
    }
    fn set_parameter(&mut self, _: &str, _: serde_json::Value) -> audio_mixer_backend::Result<()> {
        Ok(())
    }
}
fn configured_model_session(
    directory: &Path,
) -> (ResidentSession, Arc<ModelGate>, mpsc::Receiver<String>) {
    let mut active = isolated_session(directory);
    let backend = active.session.backend().clone();
    let mut descriptor = backend
        .plugins
        .list_plugins()
        .into_iter()
        .find(|d| d.input_ports.len() == 1 && d.output_ports.len() == 1)
        .unwrap();
    descriptor.plugin_id = "test.resident.model".into();
    descriptor.parameters = serde_json::from_value(serde_json::json!([
        {"id":"model", "label":"モデル", "kind":"string", "default":"old", "restart_required":true},
        {"id":"gain", "label":"音量", "kind":"number", "default":1.0}
    ]))
    .unwrap();
    let (entered, receiver) = mpsc::channel();
    let gate = Arc::new(ModelGate {
        steps: std::sync::Mutex::default(),
        entered,
    });
    backend
        .plugins
        .register_factory(Arc::new(ModelFactory {
            descriptor: descriptor.clone(),
            gate: gate.clone(),
        }))
        .unwrap();
    let mut node = backend.controller.snapshot().nodes[&active.rvc].clone();
    if let NodeProperties::TransformPlugin {
        plugin_id,
        parameters,
        ..
    } = &mut node.properties
    {
        *plugin_id = descriptor.plugin_id.clone();
        *parameters = descriptor.default_parameters();
    }
    backend.controller.update_node(node).unwrap();
    // configureの同値判定だけを通す。デバイスや音声クロックは起動しない。
    let mut microphone = backend.controller.snapshot().nodes[&active.microphone].clone();
    microphone.runtime_enabled = true;
    backend.controller.update_node(microphone).unwrap();
    active.settings.rvc_parameters = serde_json::to_value(descriptor.default_parameters()).unwrap();
    std::fs::create_dir_all(directory.join("rvc-cuda")).unwrap();
    write_json(&directory.join("rvc-cuda/plugin.json"), &descriptor).unwrap();
    active.set_mode(Mode::Rvc).unwrap();
    wait_for_rvc(&mut active);
    (active, gate, receiver)
}
fn gate_model(gate: &ModelGate, model: &str, fail: bool) -> mpsc::Sender<()> {
    let (sender, release) = mpsc::channel();
    gate.steps.lock().unwrap().push_back(ModelStep {
        model: model.into(),
        release,
        fail,
    });
    sender
}

fn assert_selected(active: &ResidentSession, mode: Mode) {
    let graph = active.session.backend().controller.snapshot();
    let selector = graph
        .nodes
        .values()
        .find(|n| n.display_name == OUTPUT_SELECTOR)
        .unwrap();
    let NodeProperties::InputSelector { config, mute, .. } = &selector.properties else {
        unreachable!()
    };
    assert_eq!(*mute, mode == Mode::Muted);
    assert_eq!(
        config.active_port_id,
        Some(selector.input_ports[usize::from(mode.is_rvc())].id)
    );
    assert!(graph
        .edges
        .iter()
        .filter(|edge| edge.to_node == active.rvc)
        .all(|edge| edge.mute == !mode.is_rvc()));
}

#[test]
fn disabling_rvc_during_prepare_unloads_without_activating_or_retrying() {
    let directory = tempfile::tempdir().unwrap();
    let mut active = isolated_session(directory.path());
    let release = gated_preparation(&mut active, false);
    active.set_mode(Mode::Rvc).unwrap();
    active.settings.rvc_enabled = false;
    active.set_mode(Mode::Direct).unwrap();
    active.unload_rvc().unwrap();
    assert!(active.preparation.pending());
    assert!(!active.rvc_wanted);
    release.send(()).unwrap();
    wait_for_rvc(&mut active);
    assert_eq!(active.rvc_starts, 0);
    assert!(!active.session.backend().nodes.is_active(active.rvc));
    active.set_mode(Mode::Rvc).unwrap();
    assert_eq!(active.mode, Mode::Direct);
    assert!(!active.preparation.pending());
    assert!(active.rvc_retry.is_none());
    active.set_mode(Mode::Muted).unwrap();
    assert!(!active.session.backend().nodes.is_active(active.microphone));
    assert!(
        !active.session.backend().controller.snapshot().nodes[&active.microphone].runtime_enabled
    );
}
#[test]
fn preparing_rvc_allows_mic_and_stop_and_preserves_latest_route_on_completion() {
    for selected in [Mode::Direct, Mode::Muted] {
        let directory = tempfile::tempdir().unwrap();
        let mut active = isolated_session(directory.path());
        let release = gated_preparation(&mut active, false);
        active.set_mode(Mode::Rvc).unwrap();
        assert_eq!(active.phase(), "starting");
        assert_selected(&active, Mode::Muted);
        active.set_mode(Mode::Direct).unwrap();
        active.set_mode(Mode::Rvc).unwrap();
        active.set_mode(selected).unwrap();
        assert!(active.preparation.pending());
        assert_eq!(active.phase(), "running");
        assert_selected(&active, selected);
        release.send(()).unwrap();
        wait_for_rvc(&mut active);
        assert_eq!(active.mode, selected);
        assert_selected(&active, selected);
        assert_eq!(active.rvc_starts, 1);
        active.set_mode(Mode::Rvc).unwrap();
        assert!(!active.preparation.pending());
        assert_selected(&active, Mode::Rvc);
        assert_eq!(active.rvc_starts, 1);
        active.unload_rvc().unwrap();
        wait_for_rvc(&mut active);
    }
}

#[test]
fn background_prepare_failure_keeps_mic_running() {
    for selected in [Mode::Direct, Mode::Muted] {
        let directory = tempfile::tempdir().unwrap();
        let mut active = isolated_session(directory.path());
        let release = gated_preparation(&mut active, true);
        active.set_mode(Mode::Rvc).unwrap();
        active.set_mode(selected).unwrap();
        release.send(()).unwrap();
        wait_for_rvc(&mut active);
        assert_eq!(active.phase(), "running");
        assert_selected(&active, selected);
        assert_eq!(active.rvc_error.as_deref(), Some("準備失敗の試験"));
    }
}

#[test]
fn preparation_completion_honors_mic_or_stop_still_queued_in_command_channel() {
    for selected in [Mode::Direct, Mode::Muted] {
        let directory = tempfile::tempdir().unwrap();
        let mut active = isolated_session(directory.path());
        let release = gated_preparation(&mut active, false);
        active.set_mode(Mode::Rvc).unwrap();
        let (tx, rx) = mpsc::channel();
        let commands = CommandSender {
            tx,
            route_request: active.route_request.clone(),
        };
        commands.send(Command::Set(selected, 2)).unwrap();
        release.send(()).unwrap();
        wait_for_rvc(&mut active);
        assert_selected(&active, selected);
        assert!(matches!(rx.try_recv(), Ok(Command::Set(mode, 2)) if mode == selected));
        active.unload_rvc().unwrap();
        wait_for_rvc(&mut active);
    }
}

#[test]
fn releasing_pending_rvc_selects_mic_before_completion_and_discards_instance() {
    let directory = tempfile::tempdir().unwrap();
    let mut active = isolated_session(directory.path());
    let release = gated_preparation(&mut active, false);
    active.set_mode(Mode::Rvc).unwrap();
    active.release_rvc().unwrap();
    assert_selected(&active, Mode::Direct);
    release.send(()).unwrap();
    wait_for_rvc(&mut active);
    assert_selected(&active, Mode::Direct);
    assert!(!active.session.backend().nodes.is_active(active.rvc));
    assert!(active.session.backend().plugins.list_instances().is_empty());
}

#[test]
fn failed_prepare_keeps_latest_route_and_retries_once_after_backoff() {
    let directory = tempfile::tempdir().unwrap();
    let mut active = isolated_session(directory.path());
    let release = gated_preparation(&mut active, true);
    active.set_mode(Mode::Rvc).unwrap();
    release.send(()).unwrap();
    wait_for_rvc(&mut active);
    assert_eq!(active.phase(), "error");
    assert_selected(&active, Mode::Muted);
    let retry = active.rvc_retry.unwrap();
    assert!(retry > Instant::now() + Duration::from_secs(9));
    for mode in [Mode::Direct, Mode::Muted, Mode::Rvc, Mode::Direct] {
        active.set_mode(mode).unwrap();
        active.retry_rvc().unwrap();
        assert_eq!(active.rvc_retry, Some(retry));
        assert!(!active.preparation.pending());
        assert_eq!(
            active.phase(),
            if mode.is_rvc() { "error" } else { "running" }
        );
        assert_selected(&active, if mode.is_rvc() { Mode::Muted } else { mode });
    }
    // 壁時計を10秒待たず、同じ期限判定を通して背景で再準備する。
    active.rvc_retry = Some(Instant::now());
    active.retry_rvc().unwrap();
    assert!(active.preparation.pending());
    assert_eq!(active.phase(), "running");
    assert_selected(&active, Mode::Direct);
    wait_for_rvc(&mut active);
    assert!(active.rvc_error.is_none());
    assert_eq!(active.rvc_recoveries, 1);
    assert_eq!(active.rvc_starts, 1);
    active.release_rvc().unwrap();
    wait_for_rvc(&mut active);
}

#[test]
fn runtime_rvc_failure_is_isolated_recreated_and_old_events_do_not_restart_it() {
    use audio_mixer_backend::audio::{engine::AudioProcessor, format::AudioBlock};
    struct FailingRvc;
    impl AudioProcessor for FailingRvc {
        fn process_in_place(&self, _: &mut AudioBlock) -> audio_mixer_backend::Result<()> {
            Err(audio_mixer_backend::BackendError::Plugin(
                "input shared ring overflowed by 960 samples".into(),
            ))
        }
    }
    for selected in [Mode::Direct, Mode::Muted, Mode::Rvc] {
        let directory = tempfile::tempdir().unwrap();
        let mut active = isolated_session(directory.path());
        active.set_mode(Mode::Rvc).unwrap();
        wait_for_rvc(&mut active);
        active.set_mode(selected).unwrap();
        // デバイスは全て無効のまま、実DSPイベント経路だけ動かす。
        active.session.start().unwrap();
        let backend = active.session.backend().clone();
        let instance = match backend.controller.snapshot().nodes[&active.rvc].properties {
            NodeProperties::TransformPlugin { instance_id, .. } => instance_id,
            _ => unreachable!(),
        };
        let previous = backend.plugins.get(instance).unwrap();
        backend
            .runtime
            .bind_processor(active.rvc, Arc::new(FailingRvc));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            backend.controller.snapshot().nodes[&active.rvc].state,
            RuntimeState::Running
        );
        active.tick_audio().unwrap();
        assert!(active.rvc_error.as_ref().unwrap().contains("overflowed"));
        assert_eq!(
            active.phase(),
            if selected.is_rvc() {
                "error"
            } else {
                "running"
            }
        );
        assert_selected(
            &active,
            if selected.is_rvc() {
                Mode::Muted
            } else {
                selected
            },
        );
        assert!(!backend.controller.snapshot().nodes[&active.rvc].runtime_enabled);
        let retry = active.rvc_retry;
        // 旧通知を排出しても期限を延長せず、MIC/停止を止めない。
        for _ in 0..3 {
            thread::sleep(Duration::from_millis(15));
            active.tick_audio().unwrap();
            assert_eq!(active.rvc_retry, retry);
            assert_eq!(active.rvc_recoveries, 1);
        }
        active.rvc_retry = Some(Instant::now());
        active.retry_rvc().unwrap();
        assert!(active.preparation.pending());
        assert_selected(
            &active,
            if selected.is_rvc() {
                Mode::Muted
            } else {
                selected
            },
        );
        assert_eq!(
            active.phase(),
            if selected.is_rvc() {
                "starting"
            } else {
                "running"
            }
        );
        wait_for_rvc(&mut active);
        let recovered = backend.plugins.get(instance).unwrap();
        assert!(!Arc::ptr_eq(&previous, &recovered));
        assert_eq!(active.rvc_starts, 2);
        for mode in [Mode::Rvc, Mode::Direct, selected] {
            active.set_mode(mode).unwrap();
            active.tick_audio().unwrap();
            assert!(active.rvc_error.is_none());
            assert!(active.rvc_retry.is_none());
            assert_selected(&active, mode);
            assert!(Arc::ptr_eq(
                &recovered,
                &backend.plugins.get(instance).unwrap()
            ));
            assert_eq!(active.rvc_starts, 2);
        }
        // 明示解放は復旧予約も取り消し、MICで不要なモデルを再生成しない。
        active.fail_rvc("解放直前の再障害".into()).unwrap();
        active.release_rvc().unwrap();
        wait_for_rvc(&mut active);
        active.retry_rvc().unwrap();
        assert!(!active.rvc_wanted);
        assert!(active.rvc_retry.is_none());
        assert!(!backend.nodes.is_active(active.rvc));
        assert_eq!(active.rvc_starts, 2);
        active.session.stop().unwrap();
    }
}

#[test]
fn microphone_runtime_failure_is_not_hidden_by_rvc_policy() {
    use audio_mixer_backend::audio::{engine::AudioSource, format::AudioBlock};
    struct FailingMicrophone;
    impl AudioSource for FailingMicrophone {
        fn read_into(&self, _: &mut AudioBlock) -> audio_mixer_backend::Result<()> {
            Err(audio_mixer_backend::BackendError::InvalidOperation(
                "実マイクの障害".into(),
            ))
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut active = isolated_session(directory.path());
    active.set_mode(Mode::Direct).unwrap();
    active.session.start().unwrap();
    let backend = active.session.backend().clone();
    backend
        .runtime
        .bind_source(active.microphone, Arc::new(FailingMicrophone));
    let mut node = backend.controller.snapshot().nodes[&active.microphone].clone();
    node.runtime_enabled = true;
    backend.controller.update_node(node).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let error = loop {
        if let Err(error) = active.tick_audio() {
            break error;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    };
    assert!(error.contains("実マイクの障害"));
    assert!(active.rvc_error.is_none());
    assert!(active.rvc_retry.is_none());
    let _ = active.session.stop();
}

#[test]
fn reselecting_rvc_during_explicit_release_waits_for_cleanup_before_prepare() {
    let directory = tempfile::tempdir().unwrap();
    let mut active = isolated_session(directory.path());
    let release = gated_preparation(&mut active, false);
    active.set_mode(Mode::Rvc).unwrap();
    active.release_rvc().unwrap();
    active.set_mode(Mode::Rvc).unwrap();
    assert_selected(&active, Mode::Muted);
    release.send(()).unwrap();
    wait_for_rvc(&mut active);
    assert_eq!(active.phase(), "running");
    assert_selected(&active, Mode::Rvc);
    assert_eq!(active.rvc_starts, 1); // 解放を指示した旧準備結果は有効化しない。
    active.release_rvc().unwrap();
    wait_for_rvc(&mut active);
}

#[test]
fn slow_rvc_shutdown_does_not_block_mic_selection_or_audio_clock() {
    use audio_mixer_backend::audio::{engine::AudioSource, format::AudioBlock};
    use audio_mixer_backend::plugins::instance::TransformInstance;
    use std::sync::atomic::AtomicUsize;
    struct SlowStop {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl TransformInstance for SlowStop {
        fn prepare(&mut self, _: PluginPrepareContext) -> audio_mixer_backend::Result<()> {
            Ok(())
        }
        fn process(&mut self, _: &mut AudioBlock) -> audio_mixer_backend::Result<()> {
            Ok(())
        }
        fn set_parameter(
            &mut self,
            _: &str,
            _: serde_json::Value,
        ) -> audio_mixer_backend::Result<()> {
            Ok(())
        }
        fn stop(&mut self) -> audio_mixer_backend::Result<()> {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(())
        }
    }
    struct CountingSource(Arc<AtomicUsize>);
    impl AudioSource for CountingSource {
        fn read_into(&self, block: &mut AudioBlock) -> audio_mixer_backend::Result<()> {
            block.clear();
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut active = isolated_session(directory.path());
    active.set_mode(Mode::Rvc).unwrap();
    wait_for_rvc(&mut active);
    active.session.start().unwrap();
    let backend = active.session.backend().clone();
    let instance = match backend.controller.snapshot().nodes[&active.rvc].properties {
        NodeProperties::TransformPlugin { instance_id, .. } => instance_id,
        _ => unreachable!(),
    };
    let processor = backend.plugins.get(instance).unwrap();
    let (entered, stopped) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    processor
        .replace_instance(
            Box::new(SlowStop {
                entered,
                release: wait,
            }),
            processor.parameters(),
            PluginPrepareContext {
                sample_rate: 48000,
                channels: 2,
                graph_block_frames: 480,
            },
        )
        .unwrap();
    let processed = Arc::new(AtomicUsize::new(0));
    backend.runtime.bind_source(
        active.microphone,
        Arc::new(CountingSource(processed.clone())),
    );
    let mut microphone = backend.controller.snapshot().nodes[&active.microphone].clone();
    microphone.runtime_enabled = true;
    backend.controller.update_node(microphone).unwrap();
    thread::sleep(Duration::from_millis(30));
    active.fail_rvc("解放待ちの試験".into()).unwrap();
    active.rvc_retry = Some(Instant::now());
    active.retry_rvc().unwrap();
    stopped.recv_timeout(Duration::from_secs(1)).unwrap();
    let began = Instant::now();
    for mode in [Mode::Direct, Mode::Muted, Mode::Direct] {
        active.set_mode(mode).unwrap();
        active.tick_audio().unwrap();
        assert_eq!(active.phase(), "running");
        assert_selected(&active, mode);
    }
    let _ = active.snapshot(); // 診断がstopのinner lockを待たない。
    assert!(began.elapsed() < Duration::from_millis(500));
    // 停止モードでは入力も停止するため、MIC再開後の試験ソースを再接続する。
    backend.runtime.bind_source(
        active.microphone,
        Arc::new(CountingSource(processed.clone())),
    );
    let before = processed.load(Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_millis(500);
    while processed.load(Ordering::SeqCst) == before && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(processed.load(Ordering::SeqCst) > before);
    release.send(()).unwrap();
    wait_for_rvc(&mut active);
    assert_selected(&active, Mode::Direct);
    active.release_rvc().unwrap();
    wait_for_rvc(&mut active);
    active.session.stop().unwrap();
}

#[test]
fn model_change_keeps_mic_available_and_acknowledges_only_after_prepare() {
    let directory = tempfile::tempdir().unwrap();
    let (mut active, gate, entered) = configured_model_session(directory.path());
    let previous = active.settings.clone();
    let mut next = previous.clone();
    next.rvc_parameters["model"] = serde_json::json!("new");
    let release = gate_model(&gate, "new", false);
    assert!(!active.configure_request(&next, &previous).unwrap());
    assert_eq!(entered.recv_timeout(Duration::from_secs(2)).unwrap(), "new");
    assert!(active.configuration_response.is_none());
    for mode in [Mode::Direct, Mode::Muted, Mode::Direct] {
        active.set_mode(mode).unwrap();
        assert_eq!(active.phase(), "running");
        assert_selected(&active, mode);
    }
    release.send(()).unwrap();
    wait_for_rvc(&mut active);
    let response = active.configuration_response.take().unwrap();
    assert!(response.applied);
    assert_eq!(response.settings, next);
    assert_eq!(active.settings, next);
    assert_selected(&active, Mode::Direct);
    let instance = active.session.backend().plugins.list_instances()[0];
    let processor = active.session.backend().plugins.get(instance).unwrap();
    active.set_mode(Mode::Rvc).unwrap();
    let mut immediate = next.clone();
    immediate.rvc_parameters["gain"] = serde_json::json!(0.5);
    assert!(active.configure_request(&immediate, &next).unwrap());
    assert!(Arc::ptr_eq(
        &processor,
        &active.session.backend().plugins.get(instance).unwrap()
    ));
    assert!(!active.preparation.pending());
    active.release_rvc().unwrap();
    wait_for_rvc(&mut active);
}

#[test]
fn failed_model_change_waits_for_old_model_rollback_before_rejecting_settings() {
    for rollback_fails in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (mut active, gate, entered) = configured_model_session(directory.path());
        let previous = active.settings.clone();
        let mut next = previous.clone();
        next.rvc_parameters["model"] = serde_json::json!("new");
        let new_release = gate_model(&gate, "new", true);
        let old_release = gate_model(&gate, "old", rollback_fails);
        assert!(!active.configure_request(&next, &previous).unwrap());
        assert_eq!(entered.recv_timeout(Duration::from_secs(2)).unwrap(), "new");
        active.set_mode(Mode::Direct).unwrap();
        new_release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            active.tick_audio().unwrap();
            active.poll_rvc_prepare().unwrap();
            if let Ok(model) = entered.try_recv() {
                assert_eq!(model, "old");
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(active.configuration_response.is_none());
        active.set_mode(Mode::Muted).unwrap();
        assert_selected(&active, Mode::Muted);
        assert_eq!(active.phase(), "running");
        old_release.send(()).unwrap();
        wait_for_rvc(&mut active);
        let response = active.configuration_response.take().unwrap();
        assert!(!response.applied);
        assert!(response.message.contains("new のモデル読込失敗"));
        assert_eq!(response.settings, previous);
        assert_eq!(active.settings, previous);
        assert_selected(&active, Mode::Muted);
        assert_eq!(active.phase(), "running");
        assert_eq!(active.rvc_error.is_some(), rollback_fails);
        assert_eq!(active.rvc_retry.is_some(), rollback_fails);
        if rollback_fails {
            assert!(response.message.contains("旧モデルへの復旧にも失敗"));
        } else {
            assert!(active.session.backend().nodes.is_active(active.rvc));
        }
        active.release_rvc().unwrap();
        wait_for_rvc(&mut active);
    }
}

#[test]
fn releasing_during_model_change_does_not_load_an_unused_rollback_model() {
    for fails in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (mut active, gate, entered) = configured_model_session(directory.path());
        let previous = active.settings.clone();
        let mut next = previous.clone();
        next.rvc_parameters["model"] = serde_json::json!("new");
        let release = gate_model(&gate, "new", fails);
        assert!(!active.configure_request(&next, &previous).unwrap());
        assert_eq!(entered.recv_timeout(Duration::from_secs(2)).unwrap(), "new");
        active.release_rvc().unwrap();
        release.send(()).unwrap();
        wait_for_rvc(&mut active);
        let response = active.configuration_response.take().unwrap();
        assert_eq!(response.applied, !fails);
        assert_eq!(response.settings, if fails { previous } else { next });
        assert_selected(&active, Mode::Direct);
        assert!(active.session.backend().plugins.list_instances().is_empty());
        assert!(!active.rvc_wanted);
        assert!(active.rvc_retry.is_none());
        assert_eq!(active.rvc_starts, 1);
    }
}

#[test]
fn model_start_failure_restores_old_model_before_configuration_response() {
    let directory = tempfile::tempdir().unwrap();
    let (mut active, _, _) = configured_model_session(directory.path());
    let previous = active.settings.clone();
    let mut next = previous.clone();
    next.rvc_parameters["model"] = serde_json::json!("start-fail");
    assert!(!active.configure_request(&next, &previous).unwrap());
    wait_for_rvc(&mut active);
    let response = active.configuration_response.take().unwrap();
    assert!(!response.applied);
    assert!(response.message.contains("新モデルの開始失敗"));
    assert_eq!(active.settings, previous);
    assert!(active.rvc_error.is_none());
    assert_selected(&active, Mode::Rvc);
    assert!(active.session.backend().nodes.is_active(active.rvc));
    active.release_rvc().unwrap();
    wait_for_rvc(&mut active);
}

#[test]
fn three_outputs_resolve_only_enabled_devices_and_deduplicate_in_priority_order() {
    let directory = tempfile::tempdir().unwrap();
    let mut settings = isolated_settings(directory.path());
    assert_eq!(
        enabled_outputs_with(&settings, Mode::Direct, |_| panic!("OFFは解決しない")),
        [false; 3]
    );
    for mask in 0..8 {
        settings.cable_enabled = mask & 1 != 0;
        settings.voice_input_enabled = mask & 2 != 0;
        settings.monitoring = mask & 4 != 0;
        let expected = [
            settings.cable_enabled,
            settings.voice_input_enabled,
            settings.monitoring,
        ];
        assert_eq!(
            enabled_outputs_with(&settings, Mode::Direct, |id| Ok(id.into())),
            expected
        );
        let mut first = true;
        let deduplicated = expected.map(|enabled| {
            let result = enabled && first;
            if enabled {
                first = false;
            }
            result
        });
        assert_eq!(
            enabled_outputs_with(&settings, Mode::Direct, |_| Ok("shared".into())),
            deduplicated
        );
    }
    settings.cable_enabled = false;
    settings.voice_input_enabled = true;
    settings.monitoring = false;
    settings.voice_input_id = settings.speaker_id.clone();
    assert!(
        validate_speaker_route_with(&settings, Mode::SpeakerRvc, |id| {
            assert_ne!(id, settings.cable_id);
            assert_ne!(id, settings.monitor_id);
            Ok(id.into())
        })
        .unwrap_err()
        .contains("出力2（音声入力用）が同じ")
    );
    settings.voice_input_enabled = false;
    validate_speaker_route_with(&settings, Mode::SpeakerRvc, |id| {
        assert_eq!(id, settings.speaker_id);
        Ok(id.into())
    })
    .unwrap();
}
#[test]
#[ignore = "実マイク・CABLE-C/D・CUDAを使うSPK-RVC試験。VOICEMIXER_MIC_TEST_CONFIGが必要"]
fn live_speaker_input_shares_rvc_and_stops_feedback() {
    assert!(
        !crate::ui::send_existing(crate::ui::SHOW).unwrap(),
        "常駐を終了してください"
    );
    let config = std::env::var("VOICEMIXER_MIC_TEST_CONFIG").unwrap();
    let mut settings = Settings::load(Path::new(&config)).unwrap();
    let devices = list_devices(DeviceDirection::Render).unwrap();
    let endpoint = |name: &str| {
        devices
            .iter()
            .find(|d| d.state == "Active" && d.friendly_name.contains(name))
            .unwrap()
            .endpoint_id
            .clone()
    };
    settings.speaker_id = endpoint("CABLE-C");
    settings.speaker_name = "試験スピーカー CABLE-C".into();
    settings.cable_id = endpoint("CABLE-D");
    settings.monitor_id = settings.speaker_id.clone();
    settings.monitoring = false;
    let folder = tempfile::tempdir().unwrap();
    eprintln!("SPK実機試験: セッションを準備");
    let mut active = ResidentSession::new(&settings, folder.path(), Mode::SpeakerRvc).unwrap();
    wait_for_rvc(&mut active);
    eprintln!("SPK実機試験: 準備完了");
    let instance =
        match active.session.backend().controller.snapshot().nodes[&active.rvc].properties {
            NodeProperties::TransformPlugin { instance_id, .. } => instance_id,
            _ => unreachable!(),
        };
    let processor = active.session.backend().plugins.get(instance).unwrap();
    for mode in [
        Mode::SpeakerRvc,
        Mode::Rvc,
        Mode::Direct,
        Mode::Muted,
        Mode::SpeakerRvc,
    ] {
        eprintln!("SPK実機試験: {}へ切替", mode.label());
        active.set_mode(mode).unwrap();
        assert_eq!(
            active.meters.input,
            if mode == Mode::SpeakerRvc {
                active.speaker
            } else {
                active.microphone
            }
        );
        assert_eq!(
            active.session.backend().nodes.is_active(active.speaker),
            mode == Mode::SpeakerRvc
        );
        assert!(!active.session.backend().nodes.is_active(active.monitor));
        assert!(active
            .session
            .backend()
            .nodes
            .level_probe(active.meters.input)
            .is_some());
        assert!(Arc::ptr_eq(
            &processor,
            &active.session.backend().plugins.get(instance).unwrap()
        ));
        assert_eq!(active.rvc_starts, 1);
    }
    let assert_silent = |active: &ResidentSession| {
        assert!(active
            .session
            .backend()
            .controller
            .snapshot()
            .nodes
            .values()
            .filter(|n| matches!(n.properties, NodeProperties::InputSelector { .. }))
            .all(|n| matches!(
                n.properties,
                NodeProperties::InputSelector { mute: true, .. }
            )));
    };
    let mut monitor_collision = settings.clone();
    monitor_collision.monitoring = true;
    eprintln!("SPK実機試験: 不正なモニター設定を拒否");
    // SPK中の不正な設定適用は拒否し、有効な旧設定に戻す。
    assert!(active
        .configure(&monitor_collision)
        .unwrap_err()
        .contains("出力3（モニタ用）が同じ"));
    assert!(!active.settings.monitoring);
    // MICでは同じ設定を使えるが、SPKへの切替時はエラーで無音にする。
    active.set_mode(Mode::Direct).unwrap();
    eprintln!("SPK実機試験: MICでモニターON、その後のSPK切替を拒否");
    active.configure(&monitor_collision).unwrap();
    assert!(active.session.backend().nodes.is_active(active.monitor));
    assert!(active
        .set_mode(Mode::SpeakerRvc)
        .unwrap_err()
        .contains("出力3（モニタ用）が同じ"));
    assert_silent(&active);
    eprintln!("SPK実機試験: モニターOFFで復旧");
    active.configure(&settings).unwrap();
    active.set_mode(Mode::SpeakerRvc).unwrap();
    // OS既定の解決先が一致した場合と同じ巡回検査経路でも無音にする。
    active.settings.monitoring = true;
    eprintln!("SPK実機試験: 巡回検査と復旧");
    active.last_monitor_check = Instant::now() - Duration::from_secs(3);
    assert!(active
        .refresh_monitor_target()
        .unwrap_err()
        .contains("出力3（モニタ用）が同じ"));
    assert_silent(&active);
    active.settings.monitoring = false;
    active.set_mode(Mode::SpeakerRvc).unwrap();
    active.set_mode(Mode::Direct).unwrap();
    eprintln!("SPK実機試験: 配信用出力との一致を拒否");
    let mut collision = settings.clone();
    collision.speaker_id = settings.cable_id.clone();
    active.configure(&collision).unwrap(); // 未使用の入力はMICを妨げない。
    assert!(active.set_mode(Mode::SpeakerRvc).is_err());
    let graph = active.session.backend().controller.snapshot();
    assert!(graph
        .nodes
        .values()
        .filter(|n| matches!(n.properties, NodeProperties::InputSelector { .. }))
        .all(|n| matches!(
            n.properties,
            NodeProperties::InputSelector { mute: true, .. }
        )));
    active.configure(&settings).unwrap();
    active.set_mode(Mode::SpeakerRvc).unwrap();
    assert!(Arc::ptr_eq(
        &processor,
        &active.session.backend().plugins.get(instance).unwrap()
    ));
    eprintln!("SPK実機試験: RVC解放");
    active.release_rvc().unwrap();
    wait_for_rvc(&mut active);
    assert!(!active.session.backend().nodes.is_active(active.rvc));
    assert!(!active.session.backend().nodes.is_active(active.speaker));
    eprintln!("SPK実機試験: セッション停止");
    active.session.stop().unwrap();
    eprintln!("SPK取得/停止、観測口、MIC/SPK切替で同一モデル、循環時無音と復旧、明示解放: 成功");
}
#[test]
#[ignore = "OS既定デバイスを一時変更して復元する実機試験。VOICEMIXER_MIC_TEST_CHANGE_DEFAULTS=1が必要"]
fn follows_live_windows_defaults_without_reloading_rvc() {
    use audio_mixer_backend::windows::devices::{
        default_device_endpoints, set_default_audio_device,
        DEFAULT_COMMUNICATIONS_ENDPOINT as COMMS, DEFAULT_CONSOLE_ENDPOINT as CONSOLE,
    };
    assert_eq!(
        std::env::var("VOICEMIXER_MIC_TEST_CHANGE_DEFAULTS").as_deref(),
        Ok("1")
    );
    assert!(
        !crate::ui::send_existing(crate::ui::SHOW).unwrap(),
        "常駐を終了してください"
    );
    let config = std::env::var("VOICEMIXER_MIC_TEST_CONFIG").unwrap();
    let mut settings = Settings::load(Path::new(&config)).unwrap();
    let original = default_device_endpoints();
    let mic = original.capture.clone().unwrap();
    let inputs = list_devices(DeviceDirection::Capture).unwrap();
    let outputs = list_devices(DeviceDirection::Render).unwrap();
    let alternate_mic = inputs
        .iter()
        .find(|d| d.state == "Active" && d.friendly_name.contains("CABLE-C"))
        .expect("CABLE-C入力が必要")
        .endpoint_id
        .clone();
    let output_a = crate::config::choose_cable(&outputs)
        .unwrap()
        .endpoint_id
        .clone();
    let output_b = outputs
        .iter()
        .find(|d| d.state == "Active" && d.friendly_name.contains("CABLE-D"))
        .expect("CABLE-D出力が必要")
        .endpoint_id
        .clone();
    let enumerator = wasapi::DeviceEnumerator::new().unwrap();
    for (direction, console) in [
        (
            wasapi::Direction::Capture,
            original.capture.as_ref().unwrap(),
        ),
        (wasapi::Direction::Render, original.render.as_ref().unwrap()),
    ] {
        assert_eq!(
            &enumerator
                .get_default_device_for_role(&direction, &wasapi::Role::Multimedia)
                .unwrap()
                .get_id()
                .unwrap(),
            console,
            "ConsoleとMultimediaが異なる環境ではOS既定を変更しない"
        );
    }
    struct Restore(Vec<(String, DeviceDirection, bool)>);
    impl Restore {
        fn restore(&mut self) {
            for (id, direction, communications) in &self.0 {
                set_default_audio_device(id, *direction, *communications)
                    .expect("OS既定の復元に失敗");
            }
            self.0.clear();
        }
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            for (id, direction, communications) in &self.0 {
                if let Err(e) = set_default_audio_device(id, *direction, *communications) {
                    eprintln!("OS既定復元エラー: {e}");
                }
            }
        }
    }
    let mut restore = Restore(vec![
        (mic.clone(), DeviceDirection::Capture, false),
        (
            original.capture_communications.clone().unwrap(),
            DeviceDirection::Capture,
            true,
        ),
        (
            original.render.clone().unwrap(),
            DeviceDirection::Render,
            false,
        ),
        (
            original.render_communications.clone().unwrap(),
            DeviceDirection::Render,
            true,
        ),
    ]);
    set_default_audio_device(&output_a, DeviceDirection::Render, false).unwrap();
    set_default_audio_device(&output_b, DeviceDirection::Render, true).unwrap();
    set_default_audio_device(&alternate_mic, DeviceDirection::Capture, true).unwrap();
    assert_eq!(default_device_endpoints().render.as_ref(), Some(&output_a));
    assert_eq!(
        default_device_endpoints().render_communications.as_ref(),
        Some(&output_b)
    );
    settings.microphone_id = CONSOLE.into();
    settings.microphone_name = "既定のデバイス（録音）".into();
    settings.cable_id = CONSOLE.into();
    settings.cable_name = "既定のデバイス（再生）".into();
    settings.monitor_id = COMMS.into();
    settings.monitor_name = "既定の通信デバイス".into();
    settings.monitoring = true;
    let directory = tempfile::tempdir().unwrap();
    let mut active = ResidentSession::new(&settings, directory.path(), Mode::Rvc).unwrap();
    wait_for_rvc(&mut active);
    let graph = active.session.backend().controller.snapshot();
    let input = graph
        .nodes
        .values()
        .find(|n| matches!(n.properties, NodeProperties::Microphone { .. }))
        .unwrap()
        .id;
    let output = graph
        .nodes
        .values()
        .find(|n| n.display_name == settings.cable_name)
        .unwrap()
        .id;
    let monitor = graph
        .nodes
        .values()
        .find(|n| n.display_name == "音声モニタリング")
        .unwrap()
        .id;
    let instance = match graph.nodes[&active.rvc].properties {
        NodeProperties::TransformPlugin { instance_id, .. } => instance_id,
        _ => unreachable!(),
    };
    let processor = active.session.backend().plugins.get(instance).unwrap();
    let refresh = |active: &ResidentSession, expected: &[NodeId]| {
        let results = active
            .session
            .backend()
            .nodes
            .refresh_default_endpoint_bindings();
        let mut changed: Vec<_> = results.iter().map(|r| r.node_id).collect();
        changed.sort();
        let mut expected = expected.to_vec();
        expected.sort();
        assert_eq!(changed, expected);
        assert!(results.iter().all(|r| r.state != RuntimeState::Error));
        active
            .session
            .backend()
            .nodes
            .wait_for_device_workers(Duration::from_secs(3))
            .unwrap();
    };
    set_default_audio_device(&alternate_mic, DeviceDirection::Capture, false).unwrap();
    refresh(&active, &[input]);
    eprintln!("通常既定の入力変更：マイクだけ再接続");
    set_default_audio_device(&mic, DeviceDirection::Capture, false).unwrap();
    let timer = Instant::now();
    while timer.elapsed() < Duration::from_millis(2300) {
        active.session.tick().unwrap();
        thread::sleep(Duration::from_millis(25));
    }
    refresh(&active, &[]); // tickの2秒巡回ですでに再接続済み。
    settings.microphone_id = COMMS.into();
    active.configure(&settings).unwrap();
    set_default_audio_device(&mic, DeviceDirection::Capture, true).unwrap();
    refresh(&active, &[input]);
    eprintln!("通信既定の入力変更と2秒巡回の追従：成功");
    set_default_audio_device(&output_b, DeviceDirection::Render, false).unwrap();
    active.last_monitor_check = Instant::now() - Duration::from_secs(3);
    active.refresh_monitor_target().unwrap();
    assert!(!active.session.backend().nodes.is_active(monitor));
    refresh(&active, &[output]);
    set_default_audio_device(&output_a, DeviceDirection::Render, true).unwrap();
    active.last_monitor_check = Instant::now() - Duration::from_secs(3);
    active.refresh_monitor_target().unwrap();
    assert!(active.session.backend().nodes.is_active(monitor));
    refresh(&active, &[]);
    eprintln!("出力の通常／通信既定変更：重複送信を抑止し、異なる先への変更で復帰");
    settings.cable_id = COMMS.into();
    settings.monitor_id = CONSOLE.into();
    active.configure(&settings).unwrap();
    set_default_audio_device(&output_a, DeviceDirection::Render, false).unwrap();
    set_default_audio_device(&output_b, DeviceDirection::Render, true).unwrap();
    refresh(&active, &[output, monitor]);
    assert!(std::sync::Arc::ptr_eq(
        &processor,
        &active.session.backend().plugins.get(instance).unwrap()
    ));
    assert_eq!(active.rvc_starts, 1);
    settings.microphone_id = mic.clone();
    settings.cable_id = output_a;
    settings.monitoring = false;
    active.configure(&settings).unwrap();
    set_default_audio_device(&alternate_mic, DeviceDirection::Capture, false).unwrap();
    set_default_audio_device(&output_b, DeviceDirection::Render, false).unwrap();
    refresh(&active, &[]);
    eprintln!("個別デバイス固定とRVC保持：成功");
    active.session.stop().unwrap();
    restore.restore();
    assert_eq!(
        serde_json::to_value(default_device_endpoints()).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    eprintln!("OSの通常／通信既定をすべて元に復元済み");
}
#[test]
fn monitor_deduplicates_resolved_defaults_and_resumes_when_distinct() {
    let mut settings:Settings=serde_json::from_value(serde_json::json!({"microphone_id":"mic","microphone_name":"Mic","cable_id":"default://console","cable_name":"Default","plugin_directory":std::env::current_dir().unwrap(),"rvc_parameters":{},"last_mode":"direct","startup_mode":null,"overlay_always":true,"monitoring":true,"monitor_id":"default://communications","monitor_name":"Comms"})).unwrap();
    assert!(!monitor_enabled_with(&settings, Mode::Rvc, |_| Ok(
        "same".into()
    )));
    assert!(monitor_enabled_with(&settings, Mode::Rvc, |id| Ok(
        id.into()
    )));
    assert!(monitor_enabled_with(&settings, Mode::Rvc, |_| Err(
        "offline".into()
    )));
    settings.speaker_id = settings.monitor_id.clone();
    assert!(!monitor_enabled_with(&settings, Mode::SpeakerRvc, |id| Ok(
        id.into()
    )));
    assert!(monitor_enabled_with(&settings, Mode::Rvc, |id| Ok(
        id.into()
    )));
    assert!(
        validate_speaker_route_with(&settings, Mode::SpeakerRvc, |id| Ok(id.into()))
            .unwrap_err()
            .contains("出力3（モニタ用）が同じ")
    );
    assert!(
        validate_speaker_route_with(&settings, Mode::SpeakerRvc, |_| Ok("same".into())).is_err()
    );
    assert!(
        validate_speaker_route_with(&settings, Mode::Rvc, |_| panic!(
            "MICモードではSPKを解決しない"
        ))
        .is_ok()
    );
    settings.speaker_id = "other".into();
    assert!(monitor_enabled_with(&settings, Mode::SpeakerRvc, |id| Ok(
        id.into()
    )));
    settings.monitoring = false;
    assert!(!monitor_enabled_with(&settings, Mode::Rvc, |_| panic!(
        "OFF時にはデバイスを解決しない"
    )));
}
#[test]
fn speaker_monitor_collision_uses_resolved_ids_and_only_active_monitoring() {
    let mut settings: Settings = serde_json::from_value(serde_json::json!({
        "microphone_id":"mic","microphone_name":"Mic",
        "speaker_id":"default://console","speaker_name":"Default",
        "cable_id":"cable","cable_name":"Cable",
        "monitor_id":"default://communications","monitor_name":"Comms",
        "plugin_directory":std::env::current_dir().unwrap(),"rvc_parameters":{},
        "last_mode":"direct","startup_mode":null,"monitoring":true
    }))
    .unwrap();
    let resolve = |id: &str| {
        Ok(if id.starts_with("default://") {
            "same-speaker".into()
        } else {
            id.into()
        })
    };
    assert!(
        validate_speaker_route_with(&settings, Mode::SpeakerRvc, resolve)
            .unwrap_err()
            .contains("出力3（モニタ用）が同じ")
    );
    for mode in [Mode::Direct, Mode::Rvc, Mode::Muted] {
        validate_speaker_route_with(&settings, mode, |_| panic!("未使用のSPKを解決しない"))
            .unwrap();
    }
    settings.monitoring = false;
    validate_speaker_route_with(&settings, Mode::SpeakerRvc, |id| {
        assert_ne!(id, settings.monitor_id);
        resolve(id)
    })
    .unwrap();
    settings.monitoring = true;
    settings.monitor_id = "headphones".into();
    validate_speaker_route_with(&settings, Mode::SpeakerRvc, resolve).unwrap();
}
#[test]
#[ignore = "実マイク、追加VB-CABLE、CUDAを使う設定適用試験。VOICEMIXER_MIC_TEST_CONFIGが必要"]
fn live_settings_keep_plugin_and_rollback_failed_device_and_model() {
    assert!(
        !crate::ui::send_existing(crate::ui::SHOW).unwrap(),
        "常駐を終了してください"
    );
    let config =
        std::env::var("VOICEMIXER_MIC_TEST_CONFIG").expect("試験用設定のパスを指定してください");
    let mut settings = Settings::load(Path::new(&config)).unwrap();
    settings.monitoring = false;
    let output = list_devices(DeviceDirection::Render)
        .unwrap()
        .into_iter()
        .find(|d| {
            d.state == "Active"
                && d.friendly_name.contains("CABLE-C")
                && d.endpoint_id != settings.cable_id
        })
        .expect("試験用CABLE-Cが必要です");
    let directory = tempfile::tempdir().unwrap();
    let mut active = ResidentSession::new(&settings, directory.path(), Mode::Rvc).unwrap();
    wait_for_rvc(&mut active);
    let instance =
        match active.session.backend().controller.snapshot().nodes[&active.rvc].properties {
            NodeProperties::TransformPlugin { instance_id, .. } => instance_id,
            _ => unreachable!(),
        };
    let processor = active.session.backend().plugins.get(instance).unwrap();
    let mut changed = settings.clone();
    changed.rvc_parameters["pitch"] = serde_json::json!(11.0);
    changed.monitoring = true;
    changed.monitor_id = output.endpoint_id.clone();
    changed.monitor_name = output.friendly_name.clone();
    active.configure(&changed).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &processor,
        &active.session.backend().plugins.get(instance).unwrap()
    ));
    assert!(active
        .session
        .backend()
        .controller
        .list_nodes()
        .iter()
        .any(|n| n.display_name == "音声モニタリング"
            && n.runtime_enabled
            && active.session.backend().nodes.is_active(n.id)));
    let mut swapped = changed.clone();
    swapped.cable_id = changed.monitor_id.clone();
    swapped.cable_name = changed.monitor_name.clone();
    swapped.monitor_id = settings.cable_id.clone();
    swapped.monitor_name = settings.cable_name.clone();
    active.configure(&swapped).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &processor,
        &active.session.backend().plugins.get(instance).unwrap()
    ));
    active.configure(&settings).unwrap();
    let mut invalid = settings.clone();
    invalid.microphone_id = "{0.0.1.00000000}.{00000000-0000-0000-0000-000000000000}".into();
    let device_error = active.configure(&invalid).unwrap_err();
    eprintln!("想定した入力変更失敗: {device_error}");
    assert_eq!(active.settings, settings);
    active.session.tick().unwrap();
    invalid = settings.clone();
    invalid.rvc_parameters["model"] =
        serde_json::json!("models/__missing_for_mic_router_test__.pth");
    let model_error = active.configure(&invalid).unwrap_err();
    eprintln!("想定したモデル変更失敗: {model_error}");
    assert_eq!(active.settings, settings);
    assert!(active.session.backend().nodes.is_active(active.rvc));
    active.session.tick().unwrap();
    active.release_rvc().unwrap();
    wait_for_rvc(&mut active);
    assert!(!active.session.backend().nodes.is_active(active.rvc));
    active.session.stop().unwrap();
}
