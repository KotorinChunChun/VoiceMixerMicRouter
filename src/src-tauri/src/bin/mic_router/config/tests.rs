//! 保存互換と生成する音声経路の回帰試験。
use super::devices::choose_voice_input;
use super::*;
use audio_mixer_backend::audio::graph::NodeProperties;
use audio_mixer_backend::windows::devices::{AudioDeviceInfo, DEFAULT_COMMUNICATIONS_ENDPOINT};
use std::fs;
fn settings() -> Settings {
    Settings {
        hotkey_defaults_revision: 1,
        rvc_enabled: true,
        output_labels: default_output_labels(),
        microphone_id: "mic".into(),
        microphone_name: "Mic".into(),
        speaker_id: "speaker".into(),
        speaker_name: "Speaker".into(),
        cable_id: "cable".into(),
        cable_name: "Cable".into(),
        cable_enabled: true,
        voice_input_id: "voice-input".into(),
        voice_input_name: "Voice input".into(),
        voice_input_enabled: false,
        plugin_directory: std::env::current_dir().unwrap(),
        rvc_parameters: json!({"pitch":10}),
        last_mode: Mode::Direct,
        startup_mode: None,
        hotkey: "Ctrl+Alt+V".into(),
        hotkeys: default_hotkeys(),
        overlay_mode: OverlayMode::Always,
        monitoring: false,
        monitor_id: String::new(),
        monitor_name: String::new(),
    }
}
fn legacy_defaults() -> [Hotkey; HOTKEY_COUNT] {
    let mut keys = default_hotkeys();
    for key in &mut keys {
        key.shift = false;
        key.alt = key.key != "None";
    }
    keys
}
fn legacy_settings_json() -> Value {
    let mut settings = settings();
    settings.hotkeys = legacy_defaults();
    serde_json::to_value(settings).unwrap()
}
#[test]
fn shift_migration_is_once_preserves_custom_disabled_handling_and_conflicts() {
    let mut source = legacy_settings_json();
    source
        .as_object_mut()
        .unwrap()
        .remove("hotkey_defaults_revision");
    for key in source["hotkeys"].as_array_mut().unwrap() {
        key.as_object_mut().unwrap().remove("handling");
    }
    source["hotkeys"][0]["key"] = json!("F20");
    source["hotkeys"][0]["ctrl"] = json!(true);
    source["hotkeys"][1]["enabled"] = json!(false);
    source["hotkeys"][2] =
        json!({"enabled":true,"shift":true,"ctrl":false,"win":false,"alt":false,"key":"F9"});
    source["hotkeys"][4]["handling"] = json!("override");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    write_json(&path, &source).unwrap();
    let mut loaded = Settings::load(&path).unwrap();
    assert_eq!(loaded.hotkey_defaults_revision, 1);
    assert_eq!(loaded.hotkeys[0].label(), "Ctrl+Alt+F20");
    assert_eq!(loaded.hotkeys[0].handling, KeyHandling::Coexist);
    assert!(!loaded.hotkeys[1].enabled);
    assert!(loaded.hotkeys[1].alt && !loaded.hotkeys[1].shift);
    assert_eq!(loaded.hotkeys[4].label(), "Shift+F12");
    assert_eq!(loaded.hotkeys[4].handling, KeyHandling::Override);
    assert_eq!(loaded.hotkeys[9].label(), "Alt+F9");
    assert_eq!(loaded.hotkeys[10].label(), "Shift+F10");
    loaded.hotkeys[7].shift = false;
    loaded.hotkeys[7].alt = true;
    loaded.save(&path).unwrap();
    assert_eq!(Settings::load(&path).unwrap().hotkeys[7].label(), "Alt+F7");
}
#[test]
fn rvc_use_migrates_enabled_and_disabled_modes_never_start_rvc() {
    let mut source = serde_json::to_value(settings()).unwrap();
    source.as_object_mut().unwrap().remove("rvc_enabled");
    let mut s: Settings = serde_json::from_value(source).unwrap();
    assert!(s.rvc_enabled);
    s.rvc_enabled = false;
    for requested in [Mode::Direct, Mode::Rvc, Mode::SpeakerRvc, Mode::Muted] {
        let effective = s.available_mode(requested);
        assert_eq!(
            effective,
            if requested == Mode::Muted {
                Mode::Muted
            } else {
                Mode::Direct
            }
        );
        let graph = s.project(requested).unwrap();
        for node in graph.nodes.values() {
            if matches!(
                node.properties,
                NodeProperties::TransformPlugin { .. } | NodeProperties::EndpointLoopback { .. }
            ) {
                assert!(!node.runtime_enabled);
            }
        }
    }
    assert!(
        !serde_json::from_value::<Settings>(serde_json::to_value(s).unwrap())
            .unwrap()
            .rvc_enabled
    );
}
#[test]
fn output_labels_migrate_and_roundtrip_without_changing_routing_settings() {
    let mut original = settings();
    original.monitor_id = "headphones".into();
    original.monitor_name = "Headphones".into();
    let mut legacy = serde_json::to_value(&original).unwrap();
    legacy.as_object_mut().unwrap().remove("output_labels");
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("settings.json");
    write_json(&path, &legacy).unwrap();
    let mut loaded = Settings::load(&path).unwrap();
    assert_eq!(loaded.output_labels, ["配信用", "音声入力用", "モニタ用"]);
    loaded.output_labels = ["会議", "ゲーム & 音楽", "確認"].map(String::from);
    loaded.save(&path).unwrap();
    let reread = Settings::load(&path).unwrap();
    assert_eq!(reread.output_labels, ["会議", "ゲーム & 音楽", "確認"]);
    let mut changed = serde_json::to_value(&reread).unwrap();
    changed.as_object_mut().unwrap().remove("output_labels");
    // デバイス・有効状態・RVC値・独自キーを含む他項目は不変。
    assert_eq!(changed, legacy);
    let outputs = |value: &Settings| {
        let project = value.project(Mode::Direct).unwrap();
        let mut result: Vec<_> = project
            .nodes
            .values()
            .filter_map(|node| {
                if let NodeProperties::Monitor {
                    endpoint_id,
                    gain_db,
                    mute,
                    ..
                } = &node.properties
                {
                    Some((
                        node.display_name.clone(),
                        endpoint_id.clone(),
                        *gain_db,
                        *mute,
                        node.runtime_enabled,
                    ))
                } else {
                    None
                }
            })
            .collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    };
    assert_eq!(outputs(&original), outputs(&reread));
    for bad in ["", "  ", "改行\n名", "長".repeat(25).as_str()] {
        loaded.output_labels[1] = bad.into();
        assert!(loaded.validate().is_err());
    }
}
#[test]
fn overlay_cycle_migration_and_saved_off_are_preserved() {
    assert_eq!(OverlayMode::Off.next(), OverlayMode::Brief);
    assert_eq!(OverlayMode::Brief.next(), OverlayMode::Always);
    assert_eq!(OverlayMode::Always.next(), OverlayMode::Off);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("settings.json");
    for (old, expected) in [(false, OverlayMode::Brief), (true, OverlayMode::Always)] {
        let mut source = serde_json::to_value(settings()).unwrap();
        source.as_object_mut().unwrap().remove("overlay_mode");
        source["overlay_always"] = json!(old);
        write_json(&path, &source).unwrap();
        let mut migrated = Settings::load(&path).unwrap();
        assert_eq!(migrated.overlay_mode, expected);
        migrated.overlay_mode = OverlayMode::Off;
        migrated.save(&path).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved.get("overlay_always").is_none());
        assert_eq!(saved["overlay_mode"], "off");
        assert_eq!(
            Settings::load(&path).unwrap().overlay_mode,
            OverlayMode::Off
        );
        // 新値があるときは残存する旧値より優先する。
        let mut both = saved;
        both["overlay_always"] = json!(true);
        write_json(&path, &both).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().overlay_mode,
            OverlayMode::Off
        );
    }
}
#[test]
fn noise_toggle_uses_one_saved_plugin_parameter_and_preserves_other_settings() {
    let mut value = settings();
    let before = value.clone();
    assert!(!value.noise_suppression());
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("settings.json");
    for enabled in [true, false] {
        value.set_noise_suppression(enabled);
        value.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.noise_suppression(), enabled);
        assert_eq!(
            loaded.rvc_parameters["pitch"],
            before.rvc_parameters["pitch"]
        );
        assert_eq!(loaded.hotkeys, before.hotkeys);
        assert_eq!(loaded.microphone_id, before.microphone_id);
    }
    value.rvc_parameters[NOISE_PARAMETER] = json!("true");
    assert!(value.validate().is_err());
}
#[test]
fn routes_are_exclusive_and_rvc_never_falls_back_to_raw() {
    for mode in [Mode::Direct, Mode::Rvc, Mode::SpeakerRvc] {
        let project = settings().project(mode).unwrap();
        assert_eq!(project.nodes.len(), 8);
        assert_eq!(project.routing_edges.len(), 8);
        let mic = project
            .nodes
            .values()
            .find(|n| matches!(n.properties, NodeProperties::Microphone { .. }))
            .unwrap();
        let sink = project
            .nodes
            .values()
            .find(|n| n.display_name == "Cable")
            .unwrap();
        let selector = project
            .nodes
            .values()
            .find(|n| n.display_name == OUTPUT_SELECTOR)
            .unwrap();
        let NodeProperties::InputSelector { config, .. } = &selector.properties else {
            unreachable!()
        };
        let selected = project
            .routing_edges
            .iter()
            .find(|e| Some(e.to_port) == config.active_port_id)
            .unwrap();
        assert!(project
            .routing_edges
            .iter()
            .any(|e| e.from_node == selector.id && e.to_node == sink.id));
        let rvc = project
            .nodes
            .values()
            .find(|n| matches!(n.properties, NodeProperties::TransformPlugin { .. }))
            .unwrap();
        if mode == Mode::Direct {
            assert_eq!(selected.from_node, mic.id);
            assert!(!rvc.runtime_enabled);
        } else {
            assert_eq!(selected.from_node, rvc.id);
            let props = serde_json::to_value(&rvc.properties).unwrap();
            assert_eq!(props["bypass"], false);
            assert_eq!(props["failure_policy"], "silence");
            assert_eq!(props["switch_policy"], "silence");
            assert!(rvc.runtime_enabled);
        }
    }
}
#[test]
fn arbitrary_keys_conflicts_and_old_settings_migrate() {
    let mut s = settings();
    s.hotkeys[2].enabled = true;
    for name in ["A", "Z", "F1", "F24"] {
        s.hotkeys[2].key = name.into();
        assert!(s.validate().is_ok());
    }
    for name in ["F0", "F25", "F01", "a", "0"] {
        s.hotkeys[2].key = name.into();
        assert!(s.validate().is_err());
    }
    s.hotkeys[2].key = "V".into();
    s.hotkeys[0] = s.hotkeys[2].clone();
    assert!(s.validate().is_err());
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("old.json");
    let mut old = legacy_settings_json();
    old.as_object_mut().unwrap().remove("hotkeys");
    old["hotkey"] = json!("Alt+Shift+V");
    write_json(&path, &old).unwrap();
    let migrated = Settings::load(&path).unwrap();
    assert!(migrated.hotkeys[2].alt && migrated.hotkeys[2].shift && !migrated.hotkeys[2].ctrl);
    assert_eq!(migrated.hotkeys[2].key, "V");
    assert!(!migrated.hotkeys[0].enabled && !migrated.monitoring);
}
#[test]
fn fourteen_action_defaults_and_three_row_migration_preserve_custom_keys() {
    let defaults = default_hotkeys();
    assert_eq!(
        defaults.map(|k| k.label()),
        [
            "Shift+F5",
            "Shift+F6",
            "割当なし",
            "割当なし",
            "Shift+F12",
            "Shift+F11",
            "Shift+F8",
            "Shift+F7",
            "割当なし",
            "Shift+F9",
            "Shift+F10",
            "割当なし",
            "割当なし",
            "割当なし"
        ]
    );
    let mut old = legacy_settings_json();
    old["hotkeys"].as_array_mut().unwrap().truncate(3);
    old["hotkeys"][0]["key"] = json!("F24");
    old["hotkeys"][0]["shift"] = json!(true);
    let migrated: Settings = serde_json::from_value(old).unwrap();
    assert_eq!(migrated.hotkeys[0].label(), "Shift+Alt+F24");
    assert_eq!(migrated.hotkeys[4].label(), "Alt+F12");
    assert_eq!(migrated.hotkeys[5].label(), "Alt+F11");
    let roundtrip: Settings =
        serde_json::from_value(serde_json::to_value(&migrated).unwrap()).unwrap();
    assert_eq!(roundtrip.hotkeys, migrated.hotkeys);
}
#[test]
fn none_roundtrips_and_eight_rows_keep_custom_bindings() {
    let mut s = settings();
    for i in [2, 3, 8] {
        assert_eq!(s.hotkeys[i].key, "None");
        assert!(!s.hotkeys[i].is_assigned());
        assert_eq!(s.hotkeys[i].key_label(), "なし");
        // 手編集で有効のままでも「なし」は登録・重複判定の対象外。
        s.hotkeys[i].enabled = true;
    }
    s.validate().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("none.json");
    s.save(&path).unwrap();
    assert_eq!(Settings::load(&path).unwrap().hotkeys, s.hotkeys);
    let mut old = serde_json::to_value(s).unwrap();
    old["hotkeys"] = serde_json::to_value(legacy_defaults()).unwrap();
    old["hotkeys"][2]["enabled"] = json!(true);
    old["hotkeys"].as_array_mut().unwrap().truncate(8);
    old["hotkeys"][4]["key"] = json!("F9");
    old["hotkeys"][5]["key"] = json!("F8");
    old["hotkeys"][6]["key"] = json!("F11");
    old["hotkeys"][2]["key"] = json!("Z");
    let migrated: Settings = serde_json::from_value(old).unwrap();
    migrated.validate().unwrap();
    assert_eq!(migrated.hotkeys[4].label(), "Alt+F9");
    assert_eq!(migrated.hotkeys[5].label(), "Alt+F8");
    assert_eq!(migrated.hotkeys[2].label(), "Z");
    assert!(!migrated.hotkeys[8].is_assigned());
}
#[test]
fn six_keys_move_overlay_and_add_mute_without_resetting_custom_keys() {
    let mut old = legacy_settings_json();
    old["hotkeys"].as_array_mut().unwrap().truncate(6);
    old["hotkeys"][4]["key"] = json!("F7");
    old["hotkeys"][5]["key"] = json!("F8");
    let migrated: Settings = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(migrated.hotkeys[4].label(), "Alt+F12");
    assert_eq!(migrated.hotkeys[5].label(), "Alt+F11");
    assert_eq!(migrated.hotkeys[6].label(), "Alt+F8");
    assert_eq!(migrated.hotkeys[7].label(), "Alt+F7");
    migrated.validate().unwrap();
    old["hotkeys"][4]["key"] = json!("F12");
    old["hotkeys"][0]["key"] = json!("F7");
    let custom: Settings = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(custom.hotkeys[4].label(), "Alt+F12");
    assert_eq!(custom.hotkeys[0].label(), "Alt+F7");
    assert!(!custom.hotkeys[7].enabled);
    custom.validate().unwrap();
    old["hotkeys"][4]["key"] = json!("F7");
    old["hotkeys"][0]["key"] = json!("F10");
    let collision: Settings = serde_json::from_value(old).unwrap();
    assert_eq!(collision.hotkeys[0].label(), "Alt+F10");
    assert!(!collision.hotkeys[10].enabled);
    collision.validate().unwrap();
    let mut saved = migrated;
    saved.last_mode = Mode::Muted;
    saved.startup_mode = Some(Mode::Muted);
    saved.hotkeys[4].key = "F20".into();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("muted.json");
    saved.save(&path).unwrap();
    let loaded = Settings::load(&path).unwrap();
    assert_eq!(loaded.last_mode, Mode::Muted);
    assert_eq!(loaded.startup_mode, Some(Mode::Muted));
    assert_eq!(loaded.hotkeys[4].key, "F20");
}
#[test]
fn seven_keys_and_legacy_devices_migrate_once_without_overwriting_custom_bindings() {
    let mut old = legacy_settings_json();
    old["hotkeys"].as_array_mut().unwrap().truncate(7);
    old["hotkeys"][4]["key"] = json!("F9");
    old["hotkeys"][5]["key"] = json!("F8");
    old["hotkeys"][6]["key"] = json!("F7");
    old.as_object_mut().unwrap().remove("speaker_id");
    old.as_object_mut().unwrap().remove("speaker_name");
    let migrated: Settings = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(migrated.hotkeys, legacy_defaults());
    assert_eq!(migrated.speaker_id, DEFAULT_CONSOLE_ENDPOINT);
    assert_eq!(migrated.speaker_name, "既定のデバイス");
    old["hotkeys"][0]["key"] = json!("F9");
    old["hotkeys"][6]["key"] = json!("F24");
    let custom: Settings = serde_json::from_value(old).unwrap();
    assert_eq!(custom.hotkeys[0].label(), "Alt+F9");
    assert_eq!(custom.hotkeys[6].label(), "Alt+F24");
    assert!(!custom.hotkeys[9].enabled);
    custom.validate().unwrap();
    let roundtrip: Settings =
        serde_json::from_value(serde_json::to_value(&custom).unwrap()).unwrap();
    assert_eq!(custom, roundtrip);
    assert_eq!(Mode::parse("spk-rvc").unwrap(), Mode::SpeakerRvc);
    assert_eq!(Mode::SpeakerRvc.other(), Mode::Direct);
}
#[test]
fn muted_initial_graph_blocks_both_outputs_without_starting_rvc() {
    let mut s = settings();
    s.monitoring = true;
    s.monitor_id = "headphones".into();
    s.voice_input_enabled = true;
    let p = s.project(Mode::Muted).unwrap();
    let selector = p
        .nodes
        .values()
        .find(|n| n.display_name == OUTPUT_SELECTOR)
        .unwrap();
    assert!(matches!(
        selector.properties,
        NodeProperties::InputSelector { mute: true, .. }
    ));
    let rvc = p
        .nodes
        .values()
        .find(|n| matches!(n.properties, NodeProperties::TransformPlugin { .. }))
        .unwrap();
    assert!(!rvc.runtime_enabled);
    assert!(
        p.routing_edges
            .iter()
            .find(|e| e.to_node == rvc.id)
            .unwrap()
            .mute
    );
    for output in p
        .nodes
        .values()
        .filter(|n| matches!(n.properties, NodeProperties::Monitor { .. }))
    {
        assert!(p
            .routing_edges
            .iter()
            .filter(|e| e.to_node == output.id)
            .all(|e| e.from_node == selector.id));
    }
    assert_eq!(Mode::Muted.other(), Mode::Direct);
}
#[test]
fn rendered_samples_are_zero_in_muted_mode_on_both_outputs_and_resume() {
    use audio_mixer_backend::audio::{
        engine::{AudioProcessor, AudioRuntime, AudioSink, AudioSource, RuntimeSettings},
        format::AudioBlock,
        graph::GraphSnapshot,
    };
    use std::{
        sync::{
            atomic::{AtomicU32, AtomicU64, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    struct Signal(f32);
    impl AudioSource for Signal {
        fn read_into(&self, block: &mut AudioBlock) -> audio_mixer_backend::Result<()> {
            block.samples.fill(self.0);
            Ok(())
        }
    }
    struct Convert;
    impl AudioProcessor for Convert {
        fn process_in_place(&self, block: &mut AudioBlock) -> audio_mixer_backend::Result<()> {
            for value in &mut block.samples {
                *value *= 2.0;
            }
            Ok(())
        }
    }
    #[derive(Default)]
    struct Output {
        peak: AtomicU32,
        count: AtomicU64,
    }
    impl AudioSink for Output {
        fn write(&self, block: &AudioBlock) -> audio_mixer_backend::Result<()> {
            let peak = block.samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            self.peak.store(peak.to_bits(), Ordering::SeqCst);
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    let mut s = settings();
    s.monitoring = true;
    s.monitor_id = "headphones".into();
    s.voice_input_enabled = true;
    let p = s.project(Mode::Muted).unwrap();
    let graph = Arc::new(arc_swap::ArcSwap::from_pointee(GraphSnapshot {
        nodes: p.nodes,
        edges: p.routing_edges,
        ..Default::default()
    }));
    let runtime = AudioRuntime::new(graph.clone(), RuntimeSettings::default());
    let mut outputs = Vec::new();
    for node in graph.load().nodes.values() {
        match node.properties {
            NodeProperties::Microphone { .. } => {
                runtime.bind_source(node.id, Arc::new(Signal(0.25)))
            }
            NodeProperties::EndpointLoopback { .. } => {
                runtime.bind_source(node.id, Arc::new(Signal(0.4)))
            }
            NodeProperties::TransformPlugin { .. } => {
                runtime.bind_processor(node.id, Arc::new(Convert))
            }
            NodeProperties::Monitor { .. } => {
                let out = Arc::new(Output::default());
                runtime.bind_sink(node.id, out.clone());
                outputs.push(out);
            }
            _ => {}
        }
    }
    assert_eq!(outputs.len(), 3);
    let handle = runtime.start().unwrap();
    for (mode, expected) in [
        (Mode::Muted, 0.0),
        (Mode::Direct, 0.25),
        (Mode::Rvc, 0.5),
        (Mode::SpeakerRvc, 0.8),
        (Mode::Rvc, 0.5),
        (Mode::Muted, 0.0),
        (Mode::Direct, 0.25),
    ] {
        let mut next = (**graph.load()).clone();
        next.revision += 1;
        for node in next
            .nodes
            .values_mut()
            .filter(|n| matches!(n.properties, NodeProperties::Microphone { .. }))
        {
            node.runtime_enabled = matches!(mode, Mode::Direct | Mode::Rvc);
        }
        if mode.is_rvc() {
            for node in next.nodes.values_mut().filter(|n| {
                matches!(
                    n.properties,
                    NodeProperties::TransformPlugin { .. }
                        | NodeProperties::EndpointLoopback { .. }
                )
            }) {
                node.runtime_enabled = true;
            }
        }
        super::super::engine::select_graph(&mut next, mode).unwrap();
        graph.store(Arc::new(next));
        let counts: Vec<_> = outputs
            .iter()
            .map(|out| out.count.load(Ordering::SeqCst))
            .collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while outputs
            .iter()
            .zip(&counts)
            .any(|(out, start)| out.count.load(Ordering::SeqCst) < start + 10)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        for (out, start) in outputs.iter().zip(&counts) {
            assert!(out.count.load(Ordering::SeqCst) >= start + 10);
            assert_eq!(
                f32::from_bits(out.peak.load(Ordering::SeqCst)),
                expected,
                "{mode:?}"
            );
        }
    }
    handle.stop();
}
#[test]
fn logical_devices_precede_fixed_devices_and_survive_project_save() {
    let choices = device_choices(&[device("physical", true)], "missing", "未接続マイク");
    assert_eq!(
        choices[0],
        (DEFAULT_CONSOLE_ENDPOINT.into(), "既定のデバイス".into())
    );
    assert_eq!(
        choices[1],
        (
            DEFAULT_COMMUNICATIONS_ENDPOINT.into(),
            "既定の通信デバイス".into()
        )
    );
    assert_eq!(choices[2].0, "physical");
    assert_eq!(choices[3].0, "missing");
    let mut s = settings();
    s.microphone_id = DEFAULT_CONSOLE_ENDPOINT.into();
    s.cable_id = DEFAULT_CONSOLE_ENDPOINT.into();
    s.monitor_id = DEFAULT_COMMUNICATIONS_ENDPOINT.into();
    s.monitoring = true;
    let project = s.project(Mode::Direct).unwrap();
    assert!(project.nodes.values().any(|n|matches!(&n.properties,NodeProperties::Microphone{endpoint_id,..} if endpoint_id==DEFAULT_CONSOLE_ENDPOINT)));
    assert!(project.nodes.values().any(|n|matches!(&n.properties,NodeProperties::Monitor{endpoint_id,..} if endpoint_id==DEFAULT_COMMUNICATIONS_ENDPOINT)));
}
fn device(name: &str, default: bool) -> AudioDeviceInfo {
    serde_json::from_value(json!({"endpoint_id":name,"friendly_name":name,"direction":"capture","state":"Active",
        "is_default":default,"current_sample_rate":48000,"channels":2,"supported_sample_rates":[],"user_visible":true,"user_sort_order":0})).unwrap()
}
#[test]
fn device_choices_sort_names_after_defaults_and_keep_duplicate_ids_and_missing_selection() {
    let mut first = device("same-1", false);
    first.friendly_name = "Alpha".into();
    let mut second = device("same-2", false);
    second.friendly_name = "alpha".into();
    let mut inactive = device("hidden", false);
    inactive.state = "Disabled".into();
    let choices = device_choices(
        &[
            device("Zulu", false),
            first,
            device("あお", false),
            second,
            inactive,
        ],
        "missing",
        "Bravo",
    );
    assert_eq!(
        choices
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        vec![
            DEFAULT_CONSOLE_ENDPOINT,
            DEFAULT_COMMUNICATIONS_ENDPOINT,
            "same-1",
            "same-2",
            "missing",
            "Zulu",
            "あお"
        ]
    );
    assert_eq!(choices[4].1, "Bravo（未接続）");
    assert_eq!(device_choices(&[], "", "").len(), 2);
    assert_eq!(
        device_choices(&[], DEFAULT_CONSOLE_ENDPOINT, "既定").len(),
        2
    );
}
#[test]
fn warm_switch_preserves_nodes_and_keeps_monitor_on_selected_output() {
    let mut settings = settings();
    settings.monitoring = true;
    settings.monitor_id = "headphones".into();
    let project = settings.project(Mode::Rvc).unwrap();
    let mut graph = audio_mixer_backend::audio::graph::GraphSnapshot {
        nodes: project.nodes,
        edges: project.routing_edges,
        ..Default::default()
    };
    let rvc = graph
        .nodes
        .values()
        .find(|n| matches!(n.properties, NodeProperties::TransformPlugin { .. }))
        .unwrap()
        .clone();
    let selector = graph
        .nodes
        .values()
        .find(|n| n.display_name == OUTPUT_SELECTOR)
        .unwrap()
        .id;
    for mode in [
        Mode::Direct,
        Mode::Rvc,
        Mode::SpeakerRvc,
        Mode::Muted,
        Mode::Rvc,
        Mode::Muted,
        Mode::Direct,
    ] {
        super::super::engine::select_graph(&mut graph, mode).unwrap();
        assert_eq!(graph.nodes[&rvc.id].properties, rvc.properties);
        assert!(graph.nodes[&rvc.id].runtime_enabled);
        assert_eq!(
            graph
                .edges
                .iter()
                .find(|e| e.to_node == rvc.id)
                .unwrap()
                .mute,
            !mode.is_rvc()
        );
        assert_eq!(
            matches!(
                graph.nodes[&selector].properties,
                NodeProperties::InputSelector { mute: true, .. }
            ),
            mode == Mode::Muted
        );
        for monitor in graph
            .nodes
            .values()
            .filter(|n| matches!(n.properties, NodeProperties::Monitor { .. }))
        {
            assert!(graph
                .edges
                .iter()
                .any(|e| e.from_node == selector && e.to_node == monitor.id));
        }
    }
    settings.monitor_id = settings.cable_id.clone();
    assert!(settings.validate().is_err());
}
#[test]
fn virtual_input_and_ambiguous_cable_are_rejected() {
    assert!(choose_microphone(&[device("CABLE Output", true)]).is_err());
    assert!(choose_cable(&[device("CABLE-C Input (VB-Audio Cable C)", false)]).is_err());
    let devices = vec![
        device("CABLE In 16ch (VB-Audio Virtual Cable)", false),
        device("CABLE (VB-Audio Virtual Cable)", false),
    ];
    assert_eq!(
        choose_cable(&devices).unwrap().friendly_name,
        "CABLE (VB-Audio Virtual Cable)"
    );
}
#[test]
fn settings_replace_is_valid_and_restores_successful_mode() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("settings.json");
    let mut s = settings();
    s.save(&path).unwrap();
    s.last_mode = Mode::Rvc;
    s.save(&path).unwrap();
    assert_eq!(Settings::load(&path).unwrap().last_mode, Mode::Rvc);
}
#[test]
fn nine_keys_upgrade_once_and_preserve_custom_disabled_keys() {
    let mut old = legacy_settings_json();
    old["hotkeys"].as_array_mut().unwrap().truncate(9);
    old["hotkeys"][4]["key"] = json!("F10");
    old["hotkeys"][5]["key"] = json!("F9");
    let migrated: Settings = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(migrated.hotkeys, legacy_defaults());
    let saved: Settings = serde_json::from_value(serde_json::to_value(&migrated).unwrap()).unwrap();
    assert_eq!(saved, migrated);
    old["hotkeys"][0]["key"] = json!("F12");
    old["hotkeys"][5]["enabled"] = json!(false);
    let custom: Settings = serde_json::from_value(old).unwrap();
    assert!(!custom.hotkeys[4].enabled);
    assert_eq!(custom.hotkeys[0].label(), "Alt+F12");
    assert!(!custom.hotkeys[5].enabled);
    assert_eq!(custom.hotkeys[5].key, "F9");
    custom.validate().unwrap();
}
#[test]
fn three_outputs_keep_independent_flags_and_cable_d_is_unambiguous() {
    let mut s = settings();
    s.monitor_id = "headphones".into();
    for bits in 0..8 {
        s.cable_enabled = bits & 1 != 0;
        s.voice_input_enabled = bits & 2 != 0;
        s.monitoring = bits & 4 != 0;
        let p = s.project(Mode::Direct).unwrap();
        for (name, on) in [
            (s.cable_name.as_str(), s.cable_enabled),
            (VOICE_INPUT_OUTPUT, s.voice_input_enabled),
            ("音声モニタリング", s.monitoring),
        ] {
            assert_eq!(
                p.nodes
                    .values()
                    .find(|n| n.display_name == name)
                    .unwrap()
                    .runtime_enabled,
                on
            );
        }
    }
    let devices = [
        device("CABLE-C Input (VB-Audio Cable C)", false),
        device("CABLE-D Input (VB-Audio Cable D)", false),
    ];
    assert_eq!(
        choose_voice_input(&devices).unwrap().endpoint_id,
        devices[1].endpoint_id
    );
    assert!(choose_voice_input(&[devices[1].clone(), devices[1].clone()]).is_none());
    assert!(choose_voice_input(&devices[..1]).is_none());
    s.voice_input_id = default_voice_input_id();
    assert!(s.validate().is_err());
    s.voice_input_enabled = false;
    assert!(s.validate().is_ok());
}

#[test]
fn eleven_row_migration_preserves_custom_keys_and_defaults_ptt_to_none() {
    let mut s = settings();
    s.hotkeys[4].key = "F21".into();
    s.hotkeys[5].key = "F20".into();
    let mut json = serde_json::to_value(&s).unwrap();
    json["hotkeys"].as_array_mut().unwrap().truncate(11);
    let loaded: Settings = serde_json::from_value(json).unwrap();
    assert_eq!(&loaded.hotkeys[..11], &s.hotkeys[..11]);
    assert!(loaded.hotkeys[11..].iter().all(|k| !k.is_assigned()));
}
#[test]
fn temporary_outputs_force_on_without_changing_saved_baseline() {
    let mut s = settings();
    s.cable_enabled = false;
    s.voice_input_enabled = false;
    s.monitoring = true;
    let before = serde_json::to_value(&s).unwrap();
    let held = s.with_output_holds([true, false, true]);
    assert!(held.cable_enabled && !held.voice_input_enabled && held.monitoring);
    let released = s.with_output_holds([false; 3]);
    assert_eq!(serde_json::to_value(released).unwrap(), before);
    assert_eq!(serde_json::to_value(&s).unwrap(), before);
}

#[test]
fn ptt_masks_all_normal_outputs_and_restores_every_saved_combination() {
    for normal in 0..8 {
        let mut s = settings();
        s.cable_enabled = normal & 1 != 0;
        s.voice_input_enabled = normal & 2 != 0;
        s.monitoring = normal & 4 != 0;
        let before = serde_json::to_value(&s).unwrap();
        for pressed in 0..8 {
            let holds = [pressed & 1 != 0, pressed & 2 != 0, pressed & 4 != 0];
            let effective = s.with_output_holds(holds);
            let expected = if pressed == 0 { normal } else { pressed };
            assert_eq!(
                [
                    effective.cable_enabled,
                    effective.voice_input_enabled,
                    effective.monitoring
                ],
                [expected & 1 != 0, expected & 2 != 0, expected & 4 != 0]
            );
            assert_eq!(serde_json::to_value(&s).unwrap(), before);
        }
        assert_eq!(
            serde_json::to_value(s.with_output_holds([false; 3])).unwrap(),
            before
        );
    }
}
