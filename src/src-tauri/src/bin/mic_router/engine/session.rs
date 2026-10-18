//! 常駐中の音声経路とRVCの準備・復旧・設定トランザクションを管理する。
//!
//! ワーカーと試験はこのモジュールの子に置き、状態機械を外部へ公開しない。

#[path = "worker.rs"]
mod worker;
pub use worker::spawn;

#[cfg(all(test, windows))]
#[path = "tests.rs"]
mod tests;

use super::super::{
    config::{Mode, Settings, VOICE_INPUT_OUTPUT},
    delay,
    telemetry::{self, Collector},
};
use super::{
    complete_parameters, diagnostics,
    routing::{enabled_outputs, validate_speaker_route},
    select_graph, start,
};
use audio_mixer_backend::{
    audio::graph::{NodeId, NodeProperties, RuntimeState},
    plugins::instance::PluginPrepareContext,
    standalone::StandaloneSession,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

struct ResidentSession {
    session: StandaloneSession,
    settings: Settings,
    output_holds: [bool; 3],
    mode: Mode,
    rvc: NodeId,
    rvc_starts: u64,
    last_monitor_check: Instant,
    meters: Collector,
    delay: delay::Meter,
    delay_target: &'static str,
    monitor: NodeId,
    output: NodeId,
    voice_input: NodeId,
    microphone: NodeId,
    speaker: NodeId,
    route_request: Arc<AtomicU8>,
    preparation: Preparation,
    release_after_prepare: bool,
    release_only: bool,
    rvc_wanted: bool,
    rvc_retry: Option<Instant>,
    rvc_recoveries: u64,
    rvc_error: Option<String>,
    configuration: Option<ConfigurationTransaction>,
    configuration_response: Option<ConfigurationResponse>,
}

struct ConfigurationTransaction {
    next: Settings,
    previous: Settings,
    error: Option<String>,
}

struct ConfigurationResponse {
    settings: Settings,
    applied: bool,
    message: String,
}

const RVC_RETRY_INTERVAL: Duration = Duration::from_secs(10);

/// 準備スレッドはプラグイン解放・生成だけを行い、選択経路には触れない。
#[derive(Default)]
struct Preparation {
    worker: Option<thread::JoinHandle<Result<(), String>>>,
}
impl Preparation {
    fn pending(&self) -> bool {
        self.worker.is_some()
    }
    fn begin(&mut self, prepare: impl FnOnce() -> Result<(), String> + Send + 'static) {
        if self.worker.is_none() {
            self.worker = Some(thread::spawn(prepare));
        }
    }
    fn poll(&mut self) -> Option<Result<(), String>> {
        if !self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return None;
        }
        Some(
            self.worker
                .take()
                .unwrap()
                .join()
                .unwrap_or_else(|_| Err("RVC準備ワーカーが終了しました".into())),
        )
    }
    fn finish(&mut self) -> Result<(), String> {
        self.worker.take().map_or(Ok(()), |worker| {
            worker
                .join()
                .unwrap_or_else(|_| Err("RVC準備ワーカーが終了しました".into()))
        })
    }
}
impl ResidentSession {
    #[cfg(test)]
    fn new(settings: &Settings, directory: &Path, mode: Mode) -> Result<Self, String> {
        Self::new_with_request(settings, directory, mode, Arc::new(AtomicU8::new(0)))
    }
    fn new_with_request(
        settings: &Settings,
        directory: &Path,
        mode: Mode,
        route_request: Arc<AtomicU8>,
    ) -> Result<Self, String> {
        let mode = settings.available_mode(mode);
        // 送信停止の復元やRVC準備で、一瞬でもMICを送信してから切り替えない。
        let initial = if mode == Mode::Direct {
            Mode::Direct
        } else {
            Mode::Muted
        };
        let session = start(settings, directory, initial)?;
        let rvc = session
            .backend()
            .controller
            .list_nodes()
            .into_iter()
            .find(|n| matches!(n.properties, NodeProperties::TransformPlugin { .. }))
            .unwrap()
            .id;
        let nodes = session.backend().controller.list_nodes();
        let input = nodes
            .iter()
            .find(|n| matches!(n.properties, NodeProperties::Microphone { .. }))
            .unwrap()
            .id;
        let output = nodes
            .iter()
            .find(|n| {
                matches!(n.properties, NodeProperties::Monitor { .. })
                    && n.display_name != "音声モニタリング"
                    && n.display_name != VOICE_INPUT_OUTPUT
            })
            .unwrap()
            .id;
        let speaker = nodes
            .iter()
            .find(|n| matches!(n.properties, NodeProperties::EndpointLoopback { .. }))
            .unwrap()
            .id;
        let monitor = nodes
            .iter()
            .find(|n| n.display_name == "音声モニタリング")
            .unwrap()
            .id;
        let voice_input = nodes
            .iter()
            .find(|n| n.display_name == VOICE_INPUT_OUTPUT)
            .ok_or("音声入力用出力ノードがありません")?
            .id;
        let mut value = Self {
            session,
            settings: settings.clone(),
            output_holds: [false; 3],
            mode: initial,
            rvc,
            rvc_starts: 0,
            last_monitor_check: Instant::now(),
            meters: Collector::new(input, [output, voice_input, monitor]),
            delay: delay::Meter::default(),
            delay_target: "output",
            monitor,
            output,
            voice_input,
            microphone: input,
            speaker,
            route_request,
            preparation: Preparation::default(),
            release_after_prepare: false,
            release_only: false,
            rvc_wanted: false,
            rvc_retry: None,
            rvc_recoveries: 0,
            rvc_error: None,
            configuration: None,
            configuration_response: None,
        };
        value.set_mode(mode)?;
        Ok(value)
    }
    fn set_mode(&mut self, mode: Mode) -> Result<(), String> {
        let mode = self.settings.available_mode(mode);
        if self.mode != mode {
            self.delay.reset();
            self.meters.reset();
        }
        self.mode = mode;
        self.meters.input_enabled = mode != Mode::Muted;
        self.meters.input = if mode == Mode::SpeakerRvc {
            self.speaker
        } else {
            self.microphone
        };
        let backend = self.session.backend();
        let mut graph = backend.controller.snapshot();
        // 入力・モデル準備中と循環検出時は必ず無音にする。
        select_graph(&mut graph, Mode::Muted)?;
        backend
            .controller
            .replace_graph(graph)
            .map_err(|e| e.to_string())?;
        validate_speaker_route(&self.settings.with_output_holds(self.output_holds), mode)?;
        self.update_monitor_target()?;
        let backend = self.session.backend();
        for (id, enabled) in [
            (self.speaker, mode == Mode::SpeakerRvc),
            (self.microphone, matches!(mode, Mode::Direct | Mode::Rvc)),
        ] {
            let mut speaker = backend.controller.snapshot().nodes[&id].clone();
            if speaker.runtime_enabled != enabled || enabled && !backend.nodes.is_active(id) {
                backend.nodes.stop_node(id).map_err(|e| e.to_string())?;
                speaker.runtime_enabled = enabled;
                backend
                    .controller
                    .update_node(speaker)
                    .map_err(|e| e.to_string())?;
                if enabled {
                    backend.nodes.start_node(id).map_err(|e| e.to_string())?;
                    backend
                        .nodes
                        .wait_for_device_workers(Duration::from_secs(3))
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        if mode.is_rvc() {
            self.rvc_wanted = true;
            self.begin_rvc_prepare()?;
        }
        self.apply_selected_route()
    }
    fn begin_rvc_prepare(&mut self) -> Result<(), String> {
        if !self.settings.rvc_enabled
            || self.preparation.pending()
            || self.rvc_retry.is_some_and(|retry| Instant::now() < retry)
            || self.rvc_error.is_none() && self.session.backend().nodes.is_active(self.rvc)
        {
            return Ok(());
        }
        self.spawn_rvc_prepare()
    }
    fn spawn_rvc_prepare(&mut self) -> Result<(), String> {
        self.disable_rvc_route()?;
        self.rvc_retry = None;
        self.release_only = false;
        let backend = self.session.backend().clone();
        let rvc = self.rvc;
        let node = backend.controller.snapshot().nodes[&self.rvc].clone();
        let NodeProperties::TransformPlugin {
            instance_id,
            plugin_id,
            parameters,
            ..
        } = node.properties
        else {
            return Err("RVCノードが不正です".into());
        };
        let context = PluginPrepareContext {
            sample_rate: audio_mixer_backend::audio::format::INTERNAL_SAMPLE_RATE,
            channels: 2,
            graph_block_frames: backend.runtime.settings().block_frames,
        };
        self.preparation.begin(move || {
            // 子プロセス終了やGPUモデル生成を制御ループで待たない。
            backend.nodes.stop_node(rvc).map_err(|e| e.to_string())?;
            backend
                .plugins
                .create_instance(instance_id, &plugin_id, parameters, context)
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
        Ok(())
    }
    fn poll_rvc_prepare(&mut self) -> Result<bool, String> {
        let Some(result) = self.preparation.poll() else {
            return Ok(false);
        };
        if self.configuration.is_some() && !self.release_only {
            self.complete_configuration_stage(result)?;
            self.apply_selected_route()?;
            return Ok(true);
        }
        let released = std::mem::take(&mut self.release_only);
        if released {
            result?;
            if self.rvc_wanted {
                self.begin_rvc_prepare()?;
            }
        } else if self.release_after_prepare {
            self.release_after_prepare = false;
            self.begin_rvc_release()?;
        } else if let Err(error) = result {
            self.fail_rvc(error)?;
        } else {
            if let Err(error) = self.activate_rvc() {
                self.fail_rvc(error)?;
            }
        }
        self.apply_selected_route()?;
        Ok(true)
    }
    fn activate_rvc(&mut self) -> Result<(), String> {
        let backend = self.session.backend();
        let mut node = backend.controller.snapshot().nodes[&self.rvc].clone();
        node.runtime_enabled = true;
        backend
            .controller
            .update_node(node)
            .map_err(|e| e.to_string())?;
        backend
            .nodes
            .start_node(self.rvc)
            .map_err(|e| e.to_string())?;
        self.rvc_starts += 1;
        self.rvc_error = None;
        self.rvc_retry = None;
        Ok(())
    }
    fn apply_selected_route(&self) -> Result<(), String> {
        let backend = self.session.backend();
        // 完了通知の直前に届き、まだコマンド列から取り出していないMIC/停止も優先する。
        let mode = match self.route_request.load(Ordering::SeqCst) {
            1 => Mode::Direct,
            2 => Mode::Muted,
            _ if self.mode.is_rvc()
                && (self.preparation.pending()
                    || self.rvc_error.is_some()
                    || !backend.nodes.is_active(self.rvc)) =>
            {
                Mode::Muted
            }
            _ => self.mode,
        };
        let mut graph = backend.controller.snapshot();
        select_graph(&mut graph, mode)?;
        backend
            .controller
            .replace_graph(graph)
            .map_err(|e| e.to_string())
    }
    fn loading(&self) -> bool {
        self.mode.is_rvc() && self.preparation.pending()
    }
    fn phase(&self) -> &'static str {
        if self.loading() {
            "starting"
        } else if self.mode.is_rvc() && self.rvc_error.is_some() {
            "error"
        } else {
            "running"
        }
    }
    fn release_rvc(&mut self) -> Result<(), String> {
        self.set_mode(Mode::Direct)?;
        self.unload_rvc()
    }
    fn unload_rvc(&mut self) -> Result<(), String> {
        self.delay.reset();
        self.rvc_wanted = false;
        self.rvc_error = None;
        self.rvc_retry = None;
        self.disable_rvc_route()?;
        if self.preparation.pending() {
            // すでに解放だけの処理なら、追加解放を予約しない。
            self.release_after_prepare = !self.release_only;
            return Ok(());
        }
        self.begin_rvc_release()
    }
    fn begin_rvc_release(&mut self) -> Result<(), String> {
        self.disable_rvc_route()?;
        self.release_only = true;
        let backend = self.session.backend().clone();
        let rvc = self.rvc;
        self.preparation.begin(move || {
            backend
                .nodes
                .stop_node(rvc)
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
        Ok(())
    }
    fn disable_rvc_route(&self) -> Result<(), String> {
        let backend = self.session.backend();
        let mut graph = backend.controller.snapshot();
        // 現在のMIC/停止は維持し、RVCだけは解放より先に閉じる。
        let mode = match self.route_request.load(Ordering::SeqCst) {
            1 => Mode::Direct,
            2 => Mode::Muted,
            _ if self.mode.is_rvc() => Mode::Muted,
            _ => self.mode,
        };
        select_graph(&mut graph, mode)?;
        graph.nodes.get_mut(&self.rvc).unwrap().runtime_enabled = false;
        backend
            .controller
            .replace_graph(graph)
            .map_err(|e| e.to_string())?;
        // 解放中のprocessorへ新しい音声計画がPorts通知しないよう、先に外す。
        backend.runtime.unbind_node(self.rvc);
        Ok(())
    }
    fn fail_rvc(&mut self, error: String) -> Result<(), String> {
        self.rvc_error = Some(error);
        self.delay.reset();
        self.disable_rvc_route()?;
        if self.rvc_wanted && self.rvc_retry.is_none() {
            self.rvc_retry = Some(Instant::now() + RVC_RETRY_INTERVAL);
            self.rvc_recoveries += 1;
        }
        Ok(())
    }
    fn retry_rvc(&mut self) -> Result<(), String> {
        if self.rvc_wanted
            && !self.preparation.pending()
            && self.rvc_retry.is_some_and(|retry| Instant::now() >= retry)
        {
            self.begin_rvc_prepare()?;
        }
        Ok(())
    }
    fn tick_audio(&mut self) -> Result<(), String> {
        let mut rvc_error = None;
        let rvc = self.rvc;
        let result = self.session.tick_with_runtime_event_policy(|event| {
            self.meters.observe(event);
            if let audio_mixer_backend::audio::engine::RuntimeEvent::NodeState {
                node_id,
                state: RuntimeState::Error,
                message,
            } = event
            {
                if *node_id == rvc {
                    rvc_error.get_or_insert_with(|| {
                        message
                            .clone()
                            .unwrap_or_else(|| "RVC音声処理エラー".into())
                    });
                    return true;
                }
            }
            false
        });
        // 準備中・解放後・復旧待ちに残った旧instanceの通知は消化だけする。
        // 完了を有効化する前に必ずtickするため、新instanceを旧通知で壊さない。
        if !self.preparation.pending() && self.rvc_wanted && self.rvc_error.is_none() {
            if let Some(error) = rvc_error {
                self.fail_rvc(error)?;
            }
        }
        result.map_err(|e| e.to_string())
    }
    fn status_message(&self) -> String {
        if self.phase() == "error" {
            self.rvc_error.clone().unwrap_or_default()
        } else {
            format!("{} → {}", self.mode.label(), self.settings.cable_name)
        }
    }
    fn model_parameters(
        settings: &Settings,
    ) -> Result<std::collections::BTreeMap<String, serde_json::Value>, String> {
        serde_json::from_value(complete_parameters(settings)?).map_err(|e| format!("RVC設定: {e}"))
    }
    fn write_model_parameters(&self, settings: &Settings) -> Result<(), String> {
        let backend = self.session.backend();
        let mut node = backend.controller.snapshot().nodes[&self.rvc].clone();
        if let NodeProperties::TransformPlugin { parameters, .. } = &mut node.properties {
            *parameters = Self::model_parameters(settings)?;
        }
        backend
            .controller
            .update_node(node)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    /// trueなら同期適用済み。falseなら生成・旧モデル復元後までappliedを保留する。
    fn configure_request(&mut self, next: &Settings, previous: &Settings) -> Result<bool, String> {
        next.validate()?;
        if !next.rvc_enabled {
            self.configure(next)?;
            return Ok(true);
        }
        let backend = self.session.backend();
        let graph = backend.controller.snapshot();
        let NodeProperties::TransformPlugin {
            plugin_id,
            parameters,
            ..
        } = &graph.nodes[&self.rvc].properties
        else {
            return Err("RVCノードが不正です".into());
        };
        let next_parameters = Self::model_parameters(next)?;
        let restart = backend
            .plugins
            .descriptor(plugin_id)
            .map_err(|e| e.to_string())?
            .parameters
            .iter()
            .any(|parameter| {
                parameter.restart_required
                    && next_parameters.get(&parameter.id) != parameters.get(&parameter.id)
            });
        if !restart || !self.rvc_wanted && !backend.nodes.is_active(self.rvc) {
            self.configure(next)?;
            return Ok(true);
        }
        if self.preparation.pending() || self.configuration.is_some() {
            return Err("RVCの準備中です".into());
        }
        // デバイスの変更は従来のrollbackを使い、モデル変更だけを切り離す。
        let mut immediate = next.clone();
        immediate.rvc_parameters = self.settings.rvc_parameters.clone();
        self.configure(&immediate)?;
        self.disable_rvc_route()?;
        self.write_model_parameters(next)?;
        self.configuration = Some(ConfigurationTransaction {
            next: next.clone(),
            previous: previous.clone(),
            error: None,
        });
        self.spawn_rvc_prepare()?;
        Ok(false)
    }
    fn complete_configuration_stage(
        &mut self,
        mut result: Result<(), String>,
    ) -> Result<(), String> {
        if result.is_ok() && self.rvc_wanted && !self.release_after_prepare {
            result = self.activate_rvc();
        }
        let mut transaction = self.configuration.take().unwrap();
        if let Err(error) = result {
            if let Some(original) = transaction.error.take() {
                let message = format!("{original} / 旧モデルへの復旧にも失敗: {error}");
                self.fail_rvc(message.clone())?;
                self.configuration_response = Some(ConfigurationResponse {
                    settings: transaction.previous,
                    applied: false,
                    message,
                });
            } else {
                self.rvc_error = Some(error.clone());
                self.disable_rvc_route()?;
                self.write_model_parameters(&transaction.previous)?;
                let message = match self.configure(&transaction.previous) {
                    Ok(()) => error,
                    Err(rollback) => format!("{error} / 旧設定への復旧にも失敗: {rollback}"),
                };
                if self.rvc_wanted && !self.release_after_prepare {
                    transaction.error = Some(message);
                    self.configuration = Some(transaction);
                    self.spawn_rvc_prepare()?;
                    return Ok(());
                }
                // 明示解放の後は旧モデルも生成せず、旧設定だけを復元する。
                self.rvc_error = None;
                self.configuration_response = Some(ConfigurationResponse {
                    settings: transaction.previous,
                    applied: false,
                    message,
                });
            }
        } else {
            let applied = transaction.error.is_none();
            let message = transaction
                .error
                .unwrap_or_else(|| "設定を適用しました".into());
            let settings = if applied {
                transaction.next
            } else {
                transaction.previous
            };
            self.settings = settings.clone();
            self.rvc_error = None;
            self.rvc_retry = None;
            self.configuration_response = Some(ConfigurationResponse {
                settings,
                applied,
                message,
            });
        }
        if self.release_after_prepare || !self.rvc_wanted {
            self.release_after_prepare = false;
            self.begin_rvc_release()?;
        }
        Ok(())
    }
    fn configure(&mut self, next: &Settings) -> Result<(), String> {
        next.validate()?;
        let previous = self.settings.clone();
        if !next.rvc_enabled {
            if self.mode.is_rvc() {
                self.set_mode(Mode::Direct)?;
            }
            self.unload_rvc()?;
        }
        if previous.microphone_id != next.microphone_id
            || previous.speaker_id != next.speaker_id
            || previous.cable_id != next.cable_id
            || previous.cable_enabled != next.cable_enabled
            || previous.voice_input_id != next.voice_input_id
            || previous.voice_input_enabled != next.voice_input_enabled
            || previous.monitor_id != next.monitor_id
            || previous.monitoring != next.monitoring
            || previous.rvc_parameters != next.rvc_parameters
        {
            self.delay.reset();
        }
        if let Err(error) = self.configure_inner(next) {
            if let Err(rollback) = self.configure_inner(&previous) {
                return Err(format!("{error} / 旧設定への復旧にも失敗: {rollback}"));
            }
            return Err(error);
        }
        self.settings = next.clone();
        Ok(())
    }
    fn configure_inner(&mut self, next: &Settings) -> Result<(), String> {
        let effective = next.with_output_holds(self.output_holds);
        let next = &effective;
        validate_speaker_route(next, self.mode)?;
        let backend = self.session.backend();
        let outputs = enabled_outputs(next, self.mode);
        if self.mode == Mode::SpeakerRvc {
            let mut graph = backend.controller.snapshot();
            select_graph(&mut graph, Mode::Muted)?;
            backend
                .controller
                .replace_graph(graph)
                .map_err(|e| e.to_string())?;
        }
        // キーや表示だけの変更ではデバイスもモデルも再起動しない。
        for mut node in backend.controller.list_nodes() {
            let (endpoint, enabled) = match &node.properties {
                NodeProperties::Microphone { .. } => (
                    next.microphone_id.clone(),
                    matches!(self.mode, Mode::Direct | Mode::Rvc),
                ),
                NodeProperties::EndpointLoopback { .. } => {
                    (next.speaker_id.clone(), self.mode == Mode::SpeakerRvc)
                }
                NodeProperties::Monitor { .. } if node.display_name == "音声モニタリング" => {
                    (next.monitor_id.clone(), outputs[2])
                }
                NodeProperties::Monitor { .. } if node.display_name == VOICE_INPUT_OUTPUT => {
                    (next.voice_input_id.clone(), outputs[1])
                }
                NodeProperties::Monitor { .. } => (next.cable_id.clone(), outputs[0]),
                _ => continue,
            };
            let current = match &node.properties {
                NodeProperties::Microphone { endpoint_id, .. }
                | NodeProperties::EndpointLoopback { endpoint_id, .. }
                | NodeProperties::Monitor { endpoint_id, .. } => endpoint_id,
                _ => unreachable!(),
            };
            if *current == endpoint && node.runtime_enabled == enabled {
                continue;
            }
            backend
                .nodes
                .stop_node(node.id)
                .map_err(|e| e.to_string())?;
            match &mut node.properties {
                NodeProperties::Microphone { endpoint_id, .. }
                | NodeProperties::EndpointLoopback { endpoint_id, .. }
                | NodeProperties::Monitor { endpoint_id, .. } => *endpoint_id = endpoint,
                _ => {}
            }
            node.runtime_enabled = enabled;
            if matches!(node.properties, NodeProperties::Microphone { .. }) {
                node.display_name = next.microphone_name.clone();
            } else if matches!(node.properties, NodeProperties::EndpointLoopback { .. }) {
                node.display_name = next.speaker_name.clone();
            } else if node.display_name != "音声モニタリング"
                && node.display_name != VOICE_INPUT_OUTPUT
            {
                node.display_name = next.cable_name.clone();
            }
            backend
                .controller
                .update_node(node.clone())
                .map_err(|e| e.to_string())?;
            if enabled {
                backend
                    .nodes
                    .start_node(node.id)
                    .map_err(|e| e.to_string())?;
            }
        }
        backend
            .nodes
            .wait_for_device_workers(Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        let parameters = serde_json::from_value(complete_parameters(next)?)
            .map_err(|e| format!("RVC設定: {e}"))?;
        let graph = backend.controller.snapshot();
        if let NodeProperties::TransformPlugin {
            parameters: current,
            ..
        } = &graph.nodes[&self.rvc].properties
        {
            if *current != parameters && self.configuration.is_none() && !next.rvc_enabled {
                self.write_model_parameters(next)?;
            } else if *current != parameters && self.configuration.is_none() {
                audio_mixer_backend::commands::tauri_commands::restore_transform_settings(
                    backend, self.rvc, parameters, false,
                )
                .map_err(|e| e.to_string())?;
            }
        }
        // モデルの適用失敗後に旧パラメータを戻した場合も、その場で旧モデルを復旧する。
        if next.rvc_enabled
            && !self.preparation.pending()
            && backend.controller.snapshot().nodes[&self.rvc].runtime_enabled
            && !backend.nodes.is_active(self.rvc)
        {
            backend
                .nodes
                .start_node(self.rvc)
                .map_err(|e| e.to_string())?;
        }
        self.update_meter_target();
        self.apply_selected_route()?;
        Ok(())
    }
    fn repair(&mut self) -> Result<(), String> {
        let backend = self.session.backend();
        for node in backend.controller.list_nodes() {
            if node.id != self.rvc
                && node.runtime_enabled
                && matches!(node.state, RuntimeState::Error)
            {
                backend
                    .nodes
                    .stop_node(node.id)
                    .map_err(|e| e.to_string())?;
                backend
                    .nodes
                    .start_node(node.id)
                    .map_err(|e| e.to_string())?;
            }
        }
        self.set_mode(self.mode)
    }
    fn snapshot(&self) -> serde_json::Value {
        let mut value = diagnostics(&self.session);
        value["latency_measurement"] = self
            .delay
            .diagnostics(audio_mixer_backend::audio::clock::now_100ns());
        value["latency_measurement"]["target"] = serde_json::json!(self.delay_target);
        value["rvc_starts"] = serde_json::json!(self.rvc_starts);
        value["rvc_enabled"] = serde_json::json!(self.settings.rvc_enabled);
        value["inputs"] = serde_json::json!({
            "microphone":{"active":self.session.backend().nodes.is_active(self.microphone)},
            "speaker":{"active":self.session.backend().nodes.is_active(self.speaker)}
        });
        value["rvc_loading"] = serde_json::json!(self.preparation.pending());
        value["rvc_error"] = serde_json::json!(self.rvc_error);
        value["rvc_recoveries"] = serde_json::json!(self.rvc_recoveries);
        value["meter_events_observed"] = serde_json::json!(self.meters.observed);
        let effective = self.settings.with_output_holds(self.output_holds);
        value["push_to_talk"] = serde_json::json!(self.output_holds);
        value["monitoring"] = serde_json::json!(effective.monitoring);
        value["outputs"] = serde_json::json!([
            {"enabled":effective.cable_enabled,"active":self.session.backend().nodes.is_active(self.output)},
            {"enabled":effective.voice_input_enabled,"active":self.session.backend().nodes.is_active(self.voice_input)},
            {"enabled":effective.monitoring,"active":self.session.backend().nodes.is_active(self.monitor)}
        ]);
        value["noise_suppression"] = serde_json::json!({"enabled":effective.noise_suppression(),"active":self.mode.is_rvc() && self.settings.noise_suppression()});
        value["input_source"] = serde_json::json!(if self.mode == Mode::SpeakerRvc {
            "speaker"
        } else {
            "microphone"
        });
        value["monitor_active"] = serde_json::json!(self
            .session
            .backend()
            .controller
            .list_nodes()
            .iter()
            .any(|n| n.display_name == "音声モニタリング"
                && n.runtime_enabled
                && self.session.backend().nodes.is_active(n.id)));
        value
    }
    fn measure_delay(&mut self) -> Option<f64> {
        if !self.mode.is_rvc() || self.phase() != "running" {
            self.delay.reset();
            return None;
        }
        let backend = self.session.backend();
        let Some((output, target)) = [
            (self.monitor, "monitor"),
            (self.output, "output"),
            (self.voice_input, "voice_input"),
        ]
        .into_iter()
        .find(|(id, _)| backend.nodes.is_active(*id)) else {
            self.delay.reset();
            self.delay_target = "none";
            return None;
        };
        if self.delay_target != target {
            self.delay.reset();
            self.delay_target = target;
        }
        self.delay.update(
            backend.nodes.level_probe(self.meters.input),
            backend.nodes.level_probe(output),
            audio_mixer_backend::audio::clock::now_100ns(),
        )
    }
    fn set_output_holds(&mut self, holds: [bool; 3]) -> Result<(), String> {
        self.output_holds = holds;
        self.last_monitor_check = Instant::now() - Duration::from_secs(2);
        self.refresh_monitor_target()?;
        self.apply_selected_route()
    }
    fn refresh_monitor_target(&mut self) -> Result<(), String> {
        if self.last_monitor_check.elapsed() < Duration::from_secs(2) {
            return Ok(());
        }
        self.last_monitor_check = Instant::now();
        if let Err(error) = validate_speaker_route(
            &self.settings.with_output_holds(self.output_holds),
            self.mode,
        ) {
            let backend = self.session.backend();
            let mut graph = backend.controller.snapshot();
            select_graph(&mut graph, Mode::Muted)?;
            backend
                .controller
                .replace_graph(graph)
                .map_err(|e| e.to_string())?;
            self.delay.reset();
            return Err(error);
        }
        self.update_monitor_target()
    }
    fn update_monitor_target(&mut self) -> Result<(), String> {
        let outputs = enabled_outputs(
            &self.settings.with_output_holds(self.output_holds),
            self.mode,
        );
        let backend = self.session.backend();
        // P2Tの切替先を開始する前に、非対象の出力を停止する。
        for desired_enabled in [false, true] {
            for (id, enabled) in [self.output, self.voice_input, self.monitor]
                .into_iter()
                .zip(outputs)
            {
                if enabled != desired_enabled {
                    continue;
                }
                let mut node = backend.controller.snapshot().nodes[&id].clone();
                if node.runtime_enabled == enabled {
                    continue;
                }
                self.delay.reset();
                backend
                    .nodes
                    .stop_node(node.id)
                    .map_err(|e| e.to_string())?;
                node.runtime_enabled = enabled;
                backend
                    .controller
                    .update_node(node.clone())
                    .map_err(|e| e.to_string())?;
                if enabled {
                    backend
                        .nodes
                        .start_node(node.id)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        self.update_meter_target();
        Ok(())
    }
    fn update_meter_target(&mut self) {
        let enabled = [self.output, self.voice_input, self.monitor]
            .map(|id| self.session.backend().nodes.is_active(id));
        if self.meters.outputs_enabled != enabled {
            self.meters.outputs_enabled = enabled;
            self.meters.reset();
        }
        let next = [self.output, self.voice_input, self.monitor]
            .into_iter()
            .find(|id| self.session.backend().nodes.is_active(*id))
            .unwrap_or(self.output);
        if self.meters.output != next {
            self.meters.output = next;
            self.meters.reset();
        }
    }
}
