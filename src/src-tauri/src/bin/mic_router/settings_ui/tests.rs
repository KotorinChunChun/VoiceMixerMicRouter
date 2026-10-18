//! 設定値の検証と実機GUIの往復・配置回帰試験。
use super::super::config::{Mode, OverlayMode, APP_NAME, APP_VERSION, NOISE_PARAMETER};
use super::controls::{checked, select, selection, set_checked, set_text, text};
use super::parameters::parse_parameter;
use super::*;
use audio_mixer_backend::plugins::manifest::ParameterKind;
use windows::Win32::{Foundation::RECT, Graphics::Gdi::ClientToScreen};
fn schema() -> Vec<ParameterDescriptor> {
    let package: audio_mixer_backend::plugins::package::PluginPackageManifest =
        serde_json::from_str(include_str!(
            "../../../../../../plugins/rvc-cuda/plugin.json"
        ))
        .unwrap();
    package.descriptor.parameters
}
#[test]
fn parameters_reject_invalid_numbers_and_enum_values() {
    let schema = schema();
    for p in &schema {
        if matches!(p.kind, ParameterKind::Number | ParameterKind::Integer) {
            for bad in ["NaN", "inf", "invalid", "999999"] {
                assert!(parse_parameter(p, bad, false).is_err(), "{} {bad}", p.id);
            }
            assert!(parse_parameter(p, &p.default.to_string(), false).is_ok());
        }
        if p.kind == ParameterKind::Enum {
            assert!(parse_parameter(p, "unknown", false).is_err());
        }
    }
    let cpu = schema.iter().find(|p| p.id == "n_cpu").unwrap();
    assert!(parse_parameter(cpu, "1.5", false).is_err());
}
#[test]
#[ignore = "通常ユーザーのWindows設定画面を使用する実機GUI試験"]
fn settings_controls_roundtrip_and_page_layout() {
    let settings: Settings = serde_json::from_value(serde_json::json!({
        "microphone_id": "mic", "microphone_name": "Mic",
        "cable_id": "cable", "cable_name": "Cable", "cable_enabled": true,
        "voice_input_id": "voice-input", "voice_input_name": "Voice Input（未接続）（未接続）",
        "voice_input_enabled": false,
        "plugin_directory": std::env::current_dir().unwrap(),
        "rvc_parameters": {"pitch": 10.0}, "last_mode": "direct",
        "startup_mode": null, "overlay_always": true, "monitoring": false,
        "monitor_id": "headphones", "monitor_name": "Headphones"
    }))
    .unwrap();
    let device = |id: &str, direction: &str| {
        serde_json::from_value(serde_json::json!({"endpoint_id":id,"friendly_name":id,"direction":direction,"state":"Active","is_default":false,"current_sample_rate":48000,"channels":2,"supported_sample_rates":[],"user_visible":true,"user_sort_order":0})).unwrap()
    };
    let mut window = SettingsWindow::build(
        &settings,
        &[
            device("Zulu capture", "capture"),
            device("capture-only", "capture"),
            device("Alpha capture", "capture"),
        ],
        &[
            device("Zulu render", "render"),
            device("render-only", "render"),
            device("Alpha render", "render"),
        ],
        schema(),
        Mode::SpeakerRvc,
    )
    .unwrap();
    assert_eq!(text(window.hwnd), format!("{APP_NAME} 設定 v{APP_VERSION}"));
    assert_eq!(window.devices.len(), 5);
    for field in &window.devices {
        assert_eq!(field.devices[0].0, "default://console");
        assert_eq!(field.devices[1].0, "default://communications");
        let names: Vec<_> = field.devices[2..]
            .iter()
            .map(|(_, name)| name.to_lowercase())
            .collect();
        assert!(
            names.windows(2).all(|pair| pair[0] <= pair[1]),
            "候補が名前順になっていません: {names:?}"
        );
    }
    assert!(window.devices[0]
        .devices
        .iter()
        .any(|(id, _)| id == "capture-only"));
    assert!(!window.devices[0]
        .devices
        .iter()
        .any(|(id, _)| id == "render-only"));
    for field in &window.devices[1..] {
        assert!(field.devices.iter().any(|(id, _)| id == "render-only"));
        assert!(!field.devices.iter().any(|(id, _)| id == "capture-only"));
    }
    for size in [ICON_SMALL, ICON_BIG] {
        assert_eq!(
            unsafe { SendMessageW(window.hwnd, WM_GETICON, Some(WPARAM(size as usize)), None).0 },
            window.icon.0 as isize
        );
        assert!(!window.icon.is_invalid());
    }
    for (i, mode) in OverlayMode::ALL.into_iter().enumerate() {
        select(window.overlay, i);
        assert_eq!(window.read().unwrap().overlay_mode, mode);
    }
    let initial = window.read().unwrap();
    assert!(initial.cable_enabled);
    assert!(!initial.voice_input_enabled && !initial.monitoring);
    assert_eq!(initial.cable_id, "cable");
    assert_eq!(initial.voice_input_id, "voice-input");
    assert_eq!(initial.voice_input_name, "Voice Input");
    assert_eq!(initial.monitor_id, "headphones");
    for (i, name) in [
        (0, "Mic"),
        (2, "Cable"),
        (3, "Voice Input"),
        (4, "Headphones"),
    ] {
        assert_eq!(text(window.devices[i].handle), format!("{name}（未接続）"));
        assert_eq!(
            window.devices[i].devices[selection(window.devices[i].handle)].1,
            name
        );
    }
    assert_eq!(initial.microphone_name, settings.microphone_name);
    assert_eq!(initial.cable_name, settings.cable_name);
    assert_eq!(initial.monitor_name, settings.monitor_name);
    for (i, label) in ["会議", "ゲーム & 音楽", "確認"].into_iter().enumerate() {
        set_text(window.output_labels[i], label);
    }
    window.poll();
    let renamed = window.read().unwrap();
    assert_eq!(renamed.output_labels, ["会議", "ゲーム & 音楽", "確認"]);
    for (action, expected) in [
        (crate::actions::OUTPUT1, "出力1（会議）切替"),
        (crate::actions::OUTPUT2, "出力2（ゲーム & 音楽）切替"),
        (crate::actions::MONITOR, "出力3（確認）切替"),
    ] {
        assert_eq!(
            text(window.shortcut_titles[crate::actions::shortcut_index(action).unwrap()]),
            expected
        );
    }
    // 未保存編集は基底設定へ書き戻さない。空の用途名は画面内エラーにする。
    assert_eq!(window.base.output_labels, settings.output_labels);
    set_text(window.output_labels[0], " ");
    assert!(window.read().is_err());
    for (handle, label) in window
        .output_labels
        .into_iter()
        .zip(&settings.output_labels)
    {
        set_text(handle, label);
    }
    // 3出力の全組合せを独立して読み取り、OFFでも選択デバイスを保持する。
    for bits in 0..8 {
        for (i, handle) in window.outputs.iter().enumerate() {
            set_checked(*handle, bits & (1 << i) != 0);
        }
        let changed = window.read().unwrap();
        assert_eq!(changed.cable_enabled, bits & 1 != 0);
        assert_eq!(changed.voice_input_enabled, bits & 2 != 0);
        assert_eq!(changed.monitoring, bits & 4 != 0);
        assert_eq!(changed.cable_id, settings.cable_id);
        assert_eq!(changed.voice_input_id, settings.voice_input_id);
        assert_eq!(changed.monitor_id, settings.monitor_id);
    }
    unsafe {
        let _ = ShowWindow(window.hwnd, SW_MINIMIZE);
        assert!(IsIconic(window.hwnd).as_bool());
    }
    window.show();
    assert!(!unsafe { IsIconic(window.hwnd) }.as_bool());
    assert_eq!(unsafe { GetForegroundWindow() }, window.hwnd);
    // 既に開いている設定の再表示で、編集中の欄・選択範囲を変更しない。
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
        SetFocus(Some(window.output_labels[0])).unwrap();
        SendMessageW(
            window.output_labels[0],
            0x00b1,
            Some(WPARAM(1)),
            Some(LPARAM(2)),
        ); // EM_SETSEL
        let selection_before = SendMessageW(window.output_labels[0], 0x00b0, None, None); // EM_GETSEL
        for message in [WM_NULL, WM_TIMER, WM_PAINT, WM_APP + 72, WM_HOTKEY] {
            assert!(!dialog_message(&MSG {
                hwnd: window.hwnd,
                message,
                ..Default::default()
            }));
            assert_eq!(GetFocus(), window.output_labels[0]);
        }
        assert!(!dialog_message(&MSG {
            hwnd: HWND::default(),
            message: WM_KEYDOWN,
            wParam: WPARAM(9),
            ..Default::default()
        }));
        assert_eq!(GetFocus(), window.output_labels[0]);
        for _ in 0..12 {
            window.show();
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if !dialog_message(&message) {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            assert_eq!(GetFocus(), window.output_labels[0]);
            assert_eq!(
                SendMessageW(window.output_labels[0], 0x00b0, None, None),
                selection_before
            );
        }
        // 実際のTabキーによる項目移動は維持する。
        assert!(dialog_message(&MSG {
            hwnd: window.output_labels[0],
            message: WM_KEYDOWN,
            wParam: WPARAM(9),
            ..Default::default()
        }));
        assert_eq!(GetFocus(), window.devices[2].handle);
        // 定期メッセージを処理しながら入力欄と選択範囲が維持されることを確認する。
        SetFocus(Some(window.output_labels[0])).unwrap();
        SendMessageW(
            window.output_labels[0],
            0x00b1,
            Some(WPARAM(1)),
            Some(LPARAM(2)),
        );
        let timer = SetTimer(Some(window.hwnd), 777, 200, None);
        assert_ne!(timer, 0);
        let started = std::time::Instant::now();
        let mut focused_samples = 0;
        while started.elapsed() < std::time::Duration::from_secs(6) {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if !dialog_message(&message) {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            assert!(window.poll().is_none());
            // 別アプリへ利用者が移った場合はGetFocusがNULLになる。
            // その正当な移動は奪い返さず、設定画面が前景の標本で欄を確認する。
            if GetForegroundWindow() == window.hwnd {
                assert_eq!(GetFocus(), window.output_labels[0]);
                focused_samples += 1;
            }
            assert_eq!(
                SendMessageW(window.output_labels[0], 0x00b0, None, None),
                selection_before
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        KillTimer(Some(window.hwnd), timer).unwrap();
        assert!(focused_samples > 0, "設定画面が前景の標本がありません");
        eprintln!("定期通知中のフォーカス維持: {focused_samples}標本");
    }
    // 無効でも有効チェック自体は操作可能。修飾・キー選択は無効化する。
    let enabled =
        |h| unsafe { windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(h) }.as_bool();
    set_checked(window.rvc_enabled, false);
    ACTIONS.lock().unwrap().push(222);
    window.poll();
    assert!(enabled(window.rvc_enabled));
    assert!(window.pages[2]
        .iter()
        .filter(|h| **h != window.rvc_enabled)
        .all(|h| !enabled(*h)));
    assert!(!enabled(window.noise));
    assert!(!enabled(window.devices[1].handle));
    for index in [1, 7, 8] {
        assert!(window.keys[index].iter().all(|h| !enabled(*h)));
    }
    let disabled = window.read().unwrap();
    assert!(!disabled.rvc_enabled);
    assert_eq!(disabled.rvc_parameters, settings.rvc_parameters);
    window.busy(true);
    window.busy(false);
    assert!(enabled(window.rvc_enabled) && !enabled(window.parameters[0].handle));
    set_checked(window.rvc_enabled, true);
    ACTIONS.lock().unwrap().push(222);
    window.poll();
    assert!(window.parameters.iter().all(|p| enabled(p.handle)));
    for row in &window.keys {
        assert!(enabled(row[0]));
        for h in &row[1..] {
            assert_eq!(enabled(*h), checked(row[0]));
        }
    }
    let row = window.keys[2];
    set_checked(row[0], true);
    ACTIONS.lock().unwrap().push(HOTKEY_BASE + 20);
    assert!(window.poll().is_none());
    assert!(row.iter().all(|h| enabled(*h)));
    select(row[5], 45); // F19
    set_checked(row[1], true);
    set_checked(row[0], false);
    ACTIONS.lock().unwrap().push(HOTKEY_BASE + 20);
    window.poll();
    assert!(row[1..].iter().all(|h| !enabled(*h)));
    assert_eq!(text(row[5]), "F19");
    assert!(checked(row[1]));
    set_checked(row[0], true);
    ACTIONS.lock().unwrap().push(HOTKEY_BASE + 20);
    window.poll();
    assert!(row.iter().all(|h| enabled(*h)));
    select(row[5], 0);
    ACTIONS.lock().unwrap().push(HOTKEY_BASE + 25);
    window.poll();
    assert!(!checked(row[0]));
    assert!(row[1..].iter().all(|h| !enabled(*h)));
    assert_eq!(window.keys.len(), 14);
    for i in [2, 3, 8] {
        assert_eq!(text(window.keys[i][5]), "なし");
        assert_eq!(selection(window.keys[i][5]), 0);
    }
    for i in [0, 10] {
        set_checked(window.keys[i][0], true);
        select(window.keys[i][5], 0);
        ACTIONS.lock().unwrap().push(HOTKEY_BASE + 5 + i * 10);
        window.poll();
        assert!(!checked(window.keys[i][0]));
        assert_eq!(window.read().unwrap().hotkeys[i].key, "None");
    }
    // 表示・ノイズ・設定／入力4つと交互／出力3つと各P2T。
    let row_top = |i: usize| unsafe {
        let mut rect = RECT::default();
        GetWindowRect(window.keys[i][0], &mut rect).unwrap();
        rect.top
    };
    let display_order = [4, 8, 3, 0, 1, 7, 6, 2, 9, 11, 10, 12, 5, 13];
    for (i, pair) in display_order.windows(2).enumerate() {
        assert_eq!(
            row_top(pair[1]) - row_top(pair[0]),
            if matches!(i, 2 | 7) { 43 } else { 29 }
        );
    }
    let left = |h| unsafe {
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).unwrap();
        r.left
    };
    for row in &window.keys {
        for pair in [0, 2, 1, 4, 3, 5, 6].windows(2) {
            assert!(left(row[pair[0]]) < left(row[pair[1]]));
        }
        assert_eq!(text(row[6]), "共存");
    }
    select(window.keys[0][6], 1);
    assert_eq!(
        window.read().unwrap().hotkeys[0].handling,
        crate::config::KeyHandling::Override
    );
    for id in [260, 261] {
        let link = unsafe { GetDlgItem(Some(window.hwnd), id) }.unwrap();
        assert!(window.pages[0].contains(&link));
        assert!(text(link).contains("案内"));
    }
    for (i, row) in window.keys.iter().enumerate() {
        set_checked(row[0], true);
        select(row[5], i + 1);
    }
    let changed = window.read().unwrap();
    for (i, key) in changed.hotkeys.iter().enumerate() {
        assert_eq!(key.key, ((b'A' + i as u8) as char).to_string());
        assert!(key.enabled);
    }
    let mut last = RECT::default();
    let mut display = RECT::default();
    unsafe {
        GetWindowRect(window.keys[8][5], &mut last).unwrap();
        GetWindowRect(window.overlay, &mut display).unwrap();
    }
    // キーは別ページなので基本設定の表示方法と重ならない。
    assert!(window.pages[1].contains(&window.keys[8][5]));
    assert!(window.pages[0].contains(&window.overlay));
    assert_eq!(text(window.save), "適用");
    assert_eq!(text(window.cancel), "閉じる");
    let top = |h| {
        let mut r = RECT::default();
        unsafe {
            GetWindowRect(h, &mut r).unwrap();
        }
        r.top
    };
    assert!(top(window.overlay) < top(window.startup));
    assert!(top(window.startup) < top(window.devices[0].handle));
    assert!(top(window.devices[4].handle) < top(window.noise));
    select(window.startup, 3);
    assert_eq!(window.read().unwrap().startup_mode, Some(Mode::SpeakerRvc));
    select(window.startup, 4);
    assert_eq!(window.read().unwrap().startup_mode, Some(Mode::Muted));
    for d in &window.devices {
        assert_eq!(d.devices[0].0, "default://console");
        assert_eq!(d.devices[1].0, "default://communications");
    }
    select(window.devices[0].handle, 0);
    select(window.devices[1].handle, 0);
    select(window.devices[2].handle, 0);
    select(window.devices[3].handle, 1);
    select(window.devices[4].handle, 1);
    for output in window.outputs {
        set_checked(output, false);
    }
    let logical = window.read().unwrap();
    assert_eq!(logical.microphone_id, "default://console");
    assert_eq!(logical.speaker_id, "default://console");
    assert_eq!(logical.cable_id, "default://console");
    assert_eq!(logical.voice_input_id, "default://communications");
    assert_eq!(logical.monitor_id, "default://communications");
    assert_eq!(window.parameters.len(), 15);
    assert_eq!(
        window.read().unwrap().rvc_parameters,
        settings.rvc_parameters
    );
    for enabled in [true, false] {
        set_checked(window.noise, enabled);
        assert_eq!(window.read().unwrap().noise_suppression(), enabled);
    }
    assert!(window
        .parameters
        .iter()
        .all(|p| p.schema.id != NOISE_PARAMETER));
    assert_eq!(
        unsafe {
            SendMessageW(
                window.tab,
                windows::Win32::UI::Controls::TCM_GETITEMCOUNT,
                None,
                None,
            )
            .0
        },
        3
    );
    for page in 0..3 {
        // ネイティブタブの選択通知で可視ページと選択状態を一致させる。
        unsafe {
            SendMessageW(window.tab, TCM_SETCURSEL, Some(WPARAM(page)), None);
            let header = NMHDR {
                hwndFrom: window.tab,
                idFrom: TAB,
                code: TCN_SELCHANGE,
            };
            SendMessageW(
                window.hwnd,
                WM_NOTIFY,
                Some(WPARAM(TAB)),
                Some(LPARAM(&header as *const _ as isize)),
            );
        }
        assert!(window.poll().is_none());
        assert_eq!(
            unsafe { SendMessageW(window.tab, TCM_GETCURSEL, None, None).0 },
            page as isize
        );

        for (i, controls) in window.pages.iter().enumerate() {
            for handle in controls {
                assert_eq!(unsafe { IsWindowVisible(*handle) }.as_bool(), page == i);
            }
        }
        let mut visible = window.pages[page].clone();
        visible.extend([window.tab, window.message, window.save, window.cancel]);
        let mut client = RECT::default();
        let mut origin = windows::Win32::Foundation::POINT::default();
        unsafe {
            GetClientRect(window.hwnd, &mut client).unwrap();
            assert!(ClientToScreen(window.hwnd, &mut origin).as_bool());
        }
        assert_eq!(client.right, CLIENT_WIDTH);
        assert_eq!(client.bottom, CLIENT_HEIGHT);
        let bounds: Vec<_> = visible
            .iter()
            .map(|handle| unsafe {
                assert!(IsWindowVisible(*handle).as_bool());
                let mut rect = RECT::default();
                GetWindowRect(*handle, &mut rect).unwrap();
                assert!(rect.right > rect.left && rect.bottom > rect.top);
                assert!(
                    rect.left >= origin.x
                        && rect.top >= origin.y
                        && rect.right <= origin.x + client.right
                        && rect.bottom <= origin.y + client.bottom,
                    "画面外のコントロール: {}",
                    text(*handle)
                );
                rect
            })
            .collect();
        for (i, first) in bounds.iter().enumerate() {
            for (j, second) in bounds.iter().enumerate().skip(i + 1) {
                // タブはページの背景枠。各設定部品との重なりだけを許可する。
                if visible[i] == window.tab || visible[j] == window.tab {
                    continue;
                }
                assert!(
                    first.right <= second.left
                        || second.right <= first.left
                        || first.bottom <= second.top
                        || second.bottom <= first.top,
                    "コントロールの重なり: {} / {}",
                    text(visible[i]),
                    text(visible[j])
                );
            }
        }
    }
    set_checked(window.keys[0][0], true);
    set_checked(window.keys[0][1], true);
    set_checked(window.keys[0][3], true);
    select(window.keys[0][5], 50);
    let changed = window.read().unwrap();
    assert_eq!(changed.hotkeys[0].key, "F24");
    assert!(changed.hotkeys[0].shift && changed.hotkeys[0].win);
    set_checked(window.keys[11][0], false);
    window.busy(false);
    window.busy(true);
    assert!(
        !unsafe { windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(window.tab) }
            .as_bool()
    );
    assert!(
        !unsafe { windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(window.save) }
            .as_bool()
    );
    window.busy(false);
    assert!(!enabled(window.keys[11][5]));
    assert!(enabled(window.keys[11][0]));
    let pitch = window
        .parameters
        .iter()
        .find(|p| p.schema.id == "pitch")
        .unwrap()
        .handle;
    set_text(pitch, "NaN");
    assert!(window.read().is_err());
    set_text(pitch, "12");
    assert_eq!(window.read().unwrap().rvc_parameters["pitch"], 12.0);
    // ショートカットの応答で、関係のない未適用キー・用途名・RVC値を消さない。
    set_text(window.output_labels[0], "未適用の用途");
    let draft_key = window.read().unwrap().hotkeys[0].clone();
    let mut runtime = window.base.clone();
    runtime.overlay_mode = OverlayMode::Off;
    runtime.cable_enabled = false;
    runtime.voice_input_enabled = true;
    runtime.monitoring = false;
    runtime.set_noise_suppression(!checked(window.noise));
    window.shortcut_pending(true);
    assert!(!enabled(window.save));
    assert!(enabled(pitch));
    assert!(!window.busy);
    for action in [
        crate::actions::OVERLAY,
        crate::actions::OUTPUT1,
        crate::actions::OUTPUT2,
        crate::actions::MONITOR,
        crate::actions::NOISE,
    ] {
        window.sync_shortcut(action, &runtime);
    }
    assert!(enabled(window.save));
    let synchronized = window.read().unwrap();
    assert_eq!(synchronized.overlay_mode, runtime.overlay_mode);
    assert_eq!(
        [
            synchronized.cable_enabled,
            synchronized.voice_input_enabled,
            synchronized.monitoring
        ],
        [false, true, false]
    );
    assert_eq!(
        synchronized.noise_suppression(),
        runtime.noise_suppression()
    );
    assert_eq!(synchronized.output_labels[0], "未適用の用途");
    assert_eq!(synchronized.hotkeys[0], draft_key);
    assert_eq!(synchronized.rvc_parameters["pitch"], 12.0);
    assert!(!enabled(window.keys[11][5]));
    // なしのまま有効だけチェックして適用しても、保存結果とグレー表示を一致させる。
    set_checked(window.keys[12][0], true);
    select(window.keys[12][5], 0);
    ACTIONS.lock().unwrap().push(SAVE);
    let Some(Action::Save(applied)) = window.poll() else {
        panic!("適用要求がありません");
    };
    window.busy(true);
    window.applied(&applied);
    assert!(unsafe { IsWindowVisible(window.hwnd) }.as_bool());
    assert!(!window.busy);
    assert_eq!(window.base.rvc_parameters["pitch"], 12.0);
    assert_eq!(text(window.message), "適用しました");
    assert!(!checked(window.keys[12][0]));
    assert!(!enabled(window.keys[12][5]));
    assert!(!enabled(window.keys[11][5]));
    set_text(pitch, "13");
    ACTIONS.lock().unwrap().push(CANCEL);
    assert!(matches!(window.poll(), Some(Action::Close)));
    assert_eq!(window.base.rvc_parameters["pitch"], 12.0);
    drop(window);
    // 読み取った設定を開き直しても未接続の注記を保存名へ追加しない。
    let reopened = SettingsWindow::build(
        &initial,
        &[device("capture-only", "capture")],
        &[device("render-only", "render")],
        schema(),
        Mode::SpeakerRvc,
    )
    .unwrap();
    let reread = reopened.read().unwrap();
    assert_eq!(reread.microphone_name, initial.microphone_name);
    assert_eq!(reread.speaker_name, initial.speaker_name);
    assert_eq!(reread.cable_name, initial.cable_name);
    assert_eq!(reread.voice_input_name, initial.voice_input_name);
    assert_eq!(reread.monitor_name, initial.monitor_name);
    assert_eq!(text(reopened.devices[3].handle), "Voice Input（未接続）");
}
