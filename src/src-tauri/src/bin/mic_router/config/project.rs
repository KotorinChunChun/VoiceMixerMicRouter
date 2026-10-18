//! 常駐専用の音声グラフ生成。保存設定とは独立してノード・接続を組み立てる。
use super::{Mode, Settings};
use audio_mixer_backend::{
    audio::graph::{Edge, NodeProperties, NodeSpec, PluginInstanceId},
    config::model::PersistedConfiguration,
};
use serde_json::{json, Value};
pub const SOURCE_SELECTOR: &str = "RVC入力選択";
pub const OUTPUT_SELECTOR: &str = "モード選択";
pub const VOICE_INPUT_OUTPUT: &str = "音声入力用出力";

impl Settings {
    pub fn project(&self, mode: Mode) -> Result<PersistedConfiguration, String> {
        let mode = self.available_mode(mode);
        self.validate()?;
        let node = |name: &str, p: Value| -> Result<NodeSpec, String> {
            let p: NodeProperties = serde_json::from_value(p).map_err(|e| e.to_string())?;
            Ok(NodeSpec::new(p.node_type(), name, p))
        };
        let mut microphone = node(
            &self.microphone_name,
            json!({"kind":"microphone", "endpoint_id":self.microphone_id,
            "capture_method":"auto", "gain_db":0.0, "mute":false}),
        )?;
        microphone.runtime_enabled = matches!(mode, Mode::Direct | Mode::Rvc);
        let mut speaker = node(
            &self.speaker_name,
            json!({"kind":"endpoint_loopback", "endpoint_id":self.speaker_id,
                "filter":{"mode":"disabled","rules":[],"exe_paths":[]},
                "capture_mode":"endpoint_loopback", "gain_db":0.0, "mute":false}),
        )?;
        speaker.runtime_enabled = mode == Mode::SpeakerRvc;
        let mut source = node(
            SOURCE_SELECTOR,
            json!({"kind":"input_selector","config":{},"gain_db":0.0,"mute":!mode.is_rvc()}),
        )?;
        source.add_input_selector_port("SPK");
        if let NodeProperties::InputSelector { config, .. } = &mut source.properties {
            config.active_port_id =
                Some(source.input_ports[usize::from(mode == Mode::SpeakerRvc)].id);
        }
        let mut rvc = node(
            "RVC",
            json!({"kind":"transform_plugin", "plugin_id":"voicemixer.rvc.cuda",
                "instance_id":PluginInstanceId::new(), "parameters":self.rvc_parameters,
                "bypass":false, "failure_policy":"silence", "switch_policy":"silence", "gain_db":0.0, "mute":false}),
        )?;
        rvc.runtime_enabled = mode.is_rvc();
        let mut selector = node(
            OUTPUT_SELECTOR,
            json!({"kind":"input_selector","config":{},"gain_db":0.0,"mute":mode == Mode::Muted}),
        )?;
        selector.add_input_selector_port("RVC");
        if let NodeProperties::InputSelector { config, .. } = &mut selector.properties {
            config.active_port_id = Some(selector.input_ports[usize::from(mode.is_rvc())].id);
        }
        let mut output = node(
            &self.cable_name,
            json!({"kind":"monitor", "endpoint_id":self.cable_id, "gain_db":0.0, "mute":false}),
        )?;
        output.runtime_enabled = self.cable_enabled;
        let mut voice_input = node(
            VOICE_INPUT_OUTPUT,
            json!({"kind":"monitor", "endpoint_id":self.voice_input_id, "gain_db":0.0, "mute":false}),
        )?;
        voice_input.runtime_enabled = self.voice_input_enabled;
        let mut monitor = node(
            "音声モニタリング",
            json!({"kind":"monitor","endpoint_id":self.monitor_id,"gain_db":0.0,"mute":false}),
        )?;
        monitor.runtime_enabled = self.monitoring;
        let mut project = PersistedConfiguration::default();
        // 探索だけ行い、初めてRVCを選ぶまではプロセスを生成しない。
        project.settings.plugin_search_paths = vec![self.plugin_directory.clone()];
        project.settings.render_exclusive = false;
        let edge = |a: &NodeSpec, b: &NodeSpec, port: usize| {
            Edge::new(a.id, a.output_ports[0].id, b.id, b.input_ports[port].id)
        };
        let mut rvc_input = edge(&source, &rvc, 0);
        rvc_input.mute = !mode.is_rvc();
        project.routing_edges = vec![
            edge(&microphone, &selector, 0),
            edge(&microphone, &source, 0),
            edge(&speaker, &source, 1),
            rvc_input,
            edge(&rvc, &selector, 1),
            edge(&selector, &output, 0),
            edge(&selector, &voice_input, 0),
            edge(&selector, &monitor, 0),
        ];
        for node in [
            microphone,
            speaker,
            source,
            rvc,
            selector,
            output,
            voice_input,
            monitor,
        ] {
            project.nodes.insert(node.id, node);
        }
        Ok(project)
    }
}
