//! 設定値の検証と実機GUIの往復・配置回帰試験。
use super::super::config::{Mode, OverlayMode, NOISE_PARAMETER};
use super::controls::{checked, select, selection, set_checked, set_text, text};
use super::parameters::parse_parameter;
use super::*;
use audio_mixer_backend::plugins::manifest::ParameterKind;
use windows::Win32::{
    Foundation::{RECT, SIZE},
    Graphics::Gdi::{ClientToScreen, GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject},
};
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
    assert_eq!(
        text(window.hwnd),
        crate::window_title::WindowTitle::create("設定")
    );
    assert_eq!(window.devices.len(), 5);
    assert_eq!(window.sounds.map(selection), [1, 2, 3, 4, 5, 6, 7]);
    let sound_device = window.sound_device.as_ref().unwrap();
    assert_eq!(sound_device.devices[0].0, "default://console");
    assert_eq!(sound_device.devices[1].0, "default://communications");
    assert!(sound_device.devices.iter().any(|d| d.0 == "Alpha render"));
    select(window.sounds[0], 0);
    select(window.sounds[1], 3);
    assert_eq!(window.read().unwrap().switch_sounds, [0, 3, 3, 4, 5, 6, 7]);
    select(window.sounds[0], 1);
    select(window.sounds[1], 2);
    let sound_device = window.sound_device.as_ref().unwrap();
    let device_index = sound_device
        .devices
        .iter()
        .position(|d| d.0 == "Alpha render")
        .unwrap();
    select(sound_device.handle, device_index);
    select(window.sounds[0], 4);
    assert_eq!(window.read().unwrap().sound_device_id, "Alpha render");
    unsafe {
        SendMessageW(
            window.hwnd,
            WM_COMMAND,
            Some(WPARAM(((CBN_SELCHANGE as usize) << 16) | 500)),
            None,
        );
    }
    assert!(matches!(window.poll(), Some(Action::Preview(4, device)) if device == "Alpha render"));
    let sound_device = window.sound_device.as_ref().unwrap();
    select(sound_device.handle, 0);
    select(window.sounds[0], 1);

    for field in &window.devices {
        assert_eq!(field.devices[0], (String::new(), "未選択".into()));
        assert_eq!(field.devices[1].0, "default://console");
        assert_eq!(field.devices[2].0, "default://communications");
        let names: Vec<_> = field.devices[3..]
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
    ACTIONS.lock().unwrap().push(RVC_ENABLED);
    window.poll();
    assert!(enabled(window.rvc_enabled));
    assert!(window.pages[0].contains(&window.rvc_enabled));
    assert!(window.pages[RVC_PAGE].iter().all(|h| !enabled(*h)));
    window.page(1);
    window.page(RVC_PAGE);
    assert_eq!(window.active_page.get(), 1);
    unsafe {
        let mut rect = RECT::default();
        SendMessageW(
            window.tab,
            windows::Win32::UI::Controls::TCM_GETITEMRECT,
            Some(WPARAM(RVC_PAGE)),
            Some(LPARAM(&mut rect as *mut _ as isize)),
        );
        let position = LPARAM((((rect.top + 3) << 16) | (rect.left + 3)) as isize);
        SendMessageW(window.tab, WM_LBUTTONDOWN, None, Some(position));
        SendMessageW(window.tab, WM_LBUTTONUP, None, Some(position));
        assert_eq!(SendMessageW(window.tab, TCM_GETCURSEL, None, None).0, 1);
        SendMessageW(
            window.tab,
            WM_KEYDOWN,
            Some(WPARAM(VK_RIGHT.0 as usize)),
            None,
        );
    }
    assert!(window.poll().is_none());
    assert_eq!(window.active_page.get(), 3);
    assert_eq!(next_page(3, true, false), 1);
    assert_eq!(next_page(1, false, true), RVC_PAGE);
    window.page(0);
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
    ACTIONS.lock().unwrap().push(RVC_ENABLED);
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
    select(row[1], 3);
    set_checked(row[0], false);
    ACTIONS.lock().unwrap().push(HOTKEY_BASE + 20);
    window.poll();
    assert!(row[1..].iter().all(|h| !enabled(*h)));
    assert_eq!(text(row[5]), "F19");
    assert_eq!(selection(row[1]), 3);
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
    // 主キーなしでも左右の修飾キーだけでP2Tを設定できる。
    let ptt = window.keys[11];
    set_checked(ptt[0], true);
    select(ptt[2], 1); // 左Ctrl
    select(ptt[1], 4); // 左&右Shift
    select(ptt[5], 0);
    ACTIONS.lock().unwrap().push(HOTKEY_BASE + 115);
    window.poll();
    assert!(checked(ptt[0]));
    assert!(!enabled(ptt[6]));
    let selected = window.read().unwrap().hotkeys[11].clone();
    assert!(selected.enabled && selected.modifier_only);
    assert_eq!(selected.ctrl_side, crate::config::ModifierSide::Left);
    assert_eq!(selected.shift_side, crate::config::ModifierSide::Both);
    assert_eq!(selected.key, "None");
    set_checked(ptt[0], false);
    ACTIONS.lock().unwrap().push(HOTKEY_BASE + 110);
    window.poll();
    assert_eq!(selection(ptt[2]), 1);
    assert_eq!(selection(ptt[1]), 4);
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
    // 選択中の最長表記も矢印に隠れず、全文を読める幅を確保する。
    unsafe {
        let dc = GetDC(Some(window.hwnd));
        let old_font = SelectObject(dc, window.font.into());
        let mut narrow = Vec::new();
        let mut first_cell = RECT::default();
        GetWindowRect(window.keys[0][2], &mut first_cell).unwrap();
        for (column, heading) in [
            (2, "Ctrl"),
            (1, "Shift"),
            (4, "Alt"),
            (3, "Windows"),
            (5, "キー"),
            (6, "モード"),
        ] {
            let heading = window.pages[1]
                .iter()
                .copied()
                .find(|handle| text(*handle) == heading)
                .unwrap();
            let combo = window.keys[11][column];
            let mut title = RECT::default();
            let mut cell = RECT::default();
            GetWindowRect(heading, &mut title).unwrap();
            GetWindowRect(combo, &mut cell).unwrap();
            assert_eq!(title.left, cell.left);
            assert!(title.bottom <= cell.top);
            for row in &window.keys {
                let mut rect = RECT::default();
                GetWindowRect(row[column], &mut rect).unwrap();
                assert_eq!(rect.right - rect.left, first_cell.right - first_cell.left);
                assert_eq!(rect.bottom - rect.top, first_cell.bottom - first_cell.top);
            }
            if column >= 5 {
                continue;
            }
            let original = selection(combo);
            assert_eq!(SendMessageW(combo, CB_GETCOUNT, None, None).0, 5);
            for (index, expected) in ["不要", "左", "右", "左|右", "左&右"]
                .into_iter()
                .enumerate()
            {
                select(combo, index);
                assert_eq!(text(combo), expected);
            }
            select(combo, original);
            let label = "左&右";
            let mut size = SIZE::default();
            let chars: Vec<u16> = label.encode_utf16().collect();
            assert!(GetTextExtentPoint32W(dc, &chars, &mut size).as_bool());
            let mut rect = RECT::default();
            GetClientRect(combo, &mut rect).unwrap();
            let needed = size.cx + GetSystemMetrics(SM_CXVSCROLL) + 8;
            if rect.right - rect.left < needed {
                narrow.push(format!(
                    "{label}: {}px < {needed}px",
                    rect.right - rect.left
                ));
            }
        }
        for (row, label) in ["入力マイク(原音/RVC)", "入力スピーカー(RVC)"]
            .into_iter()
            .enumerate()
        {
            let handle = window.device_rows[row][0];
            assert_eq!(text(handle), label);
            let mut size = SIZE::default();
            let chars: Vec<u16> = label.encode_utf16().collect();
            assert!(GetTextExtentPoint32W(dc, &chars, &mut size).as_bool());
            let mut rect = RECT::default();
            GetClientRect(handle, &mut rect).unwrap();
            assert!(
                size.cx <= rect.right - rect.left,
                "{label} が欄に収まりません"
            );
        }
        SelectObject(dc, old_font);
        ReleaseDC(Some(window.hwnd), dc);
        assert!(narrow.is_empty(), "{narrow:?}");
    }
    select(window.keys[0][6], 1);
    assert_eq!(
        window.read().unwrap().hotkeys[0].handling,
        crate::config::KeyHandling::Override
    );
    let link = unsafe { GetDlgItem(Some(window.hwnd), 260) }.unwrap();
    assert!(window.pages[0].contains(&link));
    assert_eq!(text(link), "公開GitHub");
    assert!(unsafe { GetDlgItem(Some(window.hwnd), 261) }.is_err());
    let geometry = |h| unsafe {
        let mut rect = RECT::default();
        GetWindowRect(h, &mut rect).unwrap();
        (rect.left, rect.right - rect.left)
    };
    assert_eq!(geometry(window.overlay), geometry(window.devices[0].handle));
    assert_eq!(geometry(window.startup), geometry(window.devices[0].handle));
    let tips = window.tips.as_ref().unwrap();
    assert!(tips.targets.contains(&window.noise));
    assert!(tips.targets.contains(&window.rvc_enabled));
    for parameter in &window.parameters {
        assert!(tips.targets.contains(&parameter.handle));
        if parameter.schema.kind == ParameterKind::File {
            let original = text(parameter.handle);
            let path = r"C:\任意のフォルダー\別のモデル.pth";
            set_text(parameter.handle, path);
            assert_eq!(
                window.read().unwrap().rvc_parameters[&parameter.schema.id],
                path
            );
            set_text(parameter.handle, &original);
        }
    }
    for id in [450, 451] {
        let button = unsafe { GetDlgItem(Some(window.hwnd), id) }.unwrap();
        assert!(window.pages[2].contains(&button));
        assert_eq!(text(button), "フォルダーを開く");
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
        assert_eq!(d.devices[1].0, "default://console");
        assert_eq!(d.devices[2].0, "default://communications");
    }
    select(window.devices[0].handle, 1);
    select(window.devices[1].handle, 1);
    select(window.devices[2].handle, 1);
    select(window.devices[3].handle, 2);
    select(window.devices[4].handle, 2);
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
        4
    );
    for page in 0..4 {
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
                assert_eq!(
                    unsafe { IsWindowVisible(*handle) }.as_bool(),
                    page == i && *handle != window.expand_devices
                );
            }
        }
        let mut visible = window.pages[page].clone();
        visible.retain(|handle| *handle != window.expand_devices);
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
    select(window.keys[0][1], 3);
    select(window.keys[0][3], 3);
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
    // P2Tの主キーも修飾キーも未指定なら適用せず、理由を表示する。
    set_checked(window.keys[12][0], true);
    select(window.keys[12][5], 0);
    for modifier in 1..=4 {
        select(window.keys[12][modifier], 0);
    }
    ACTIONS.lock().unwrap().push(SAVE);
    assert!(window.poll().is_none());
    assert!(text(window.message).contains("修飾キー"));
    set_checked(window.keys[12][0], false);
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
    window.page(0);
    let output3 = window.devices[4].handle;
    select(output3, 0);
    ACTIONS.lock().unwrap().push(214);
    assert!(window.poll().is_none());
    assert!(unsafe { IsWindowVisible(output3) }.as_bool());
    assert!(!enabled(window.outputs[2]) && !checked(window.outputs[2]));
    let draft = window.read().unwrap();
    assert!(draft.monitor_id.is_empty());
    assert!(!window.base.monitor_id.is_empty());
    window.applied(&draft);
    assert!(!unsafe { IsWindowVisible(output3) }.as_bool());
    assert!(unsafe { IsWindowVisible(window.expand_devices) }.as_bool());
    unsafe { SendMessageW(window.expand_devices, BM_CLICK, None, None) };
    assert!(window.poll().is_none());
    assert!(unsafe { IsWindowVisible(output3) }.as_bool());

    let reset_keys = unsafe { GetDlgItem(Some(window.hwnd), RESET_KEYS as i32) }.unwrap();
    let reset_sounds = unsafe { GetDlgItem(Some(window.hwnd), RESET_SOUNDS as i32) }.unwrap();
    let sound_device = window.sound_device.as_ref().unwrap().handle;
    select(sound_device, 1);
    select(window.sounds[0], 7);
    window.page(1);
    assert!(!unsafe { IsWindowVisible(reset_sounds) }.as_bool());
    unsafe { SendMessageW(reset_keys, BM_CLICK, None, None) };
    assert!(window.poll().is_none());
    let key_reset = window.read().unwrap();
    assert_eq!(key_reset.hotkeys, crate::config::default_hotkeys());
    assert!(!enabled(window.keys[13][0]));
    assert_eq!(key_reset.switch_sounds[0], 7);
    assert_eq!(key_reset.sound_device_id, "default://communications");
    set_checked(window.keys[0][0], false);
    window.page(3);
    assert!(!unsafe { IsWindowVisible(reset_keys) }.as_bool());
    unsafe { SendMessageW(reset_sounds, BM_CLICK, None, None) };
    assert!(window.poll().is_none());
    let sounds_reset = window.read().unwrap();
    assert_eq!(sounds_reset.switch_sounds, [1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(sounds_reset.sound_device_id, "default://console");
    assert!(!sounds_reset.hotkeys[0].enabled);
    assert_eq!(sounds_reset.output_labels, key_reset.output_labels);
    assert_eq!(sounds_reset.rvc_parameters, key_reset.rvc_parameters);
    assert_eq!(window.base.hotkeys, draft.hotkeys);
    set_text(pitch, "13");
    ACTIONS.lock().unwrap().push(CANCEL);
    assert!(matches!(window.poll(), Some(Action::Close)));
    assert_eq!(window.base.rvc_parameters["pitch"], 12.0);
    drop(window);
    // 読み取った設定を開き直しても未接続の注記を保存名へ追加しない。
    let mut custom = initial.clone();
    custom.rvc_enabled = false;
    custom.rvc_parameters["pitch"] = 17.into();
    custom.rvc_parameters["model"] = "models/custom.pth".into();
    let mut default_mic: audio_mixer_backend::windows::devices::AudioDeviceInfo =
        device("capture-only", "capture");
    default_mic.is_default = true;
    let mut reopened = SettingsWindow::build(
        &custom,
        &[default_mic],
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

    // 基本設定のリセットは、RVCの値や他タブの編集を巻き戻さない。
    let reset_basic = unsafe { GetDlgItem(Some(reopened.hwnd), RESET_BASIC as i32) }.unwrap();
    select(reopened.overlay, 0);
    select(reopened.sounds[0], 7);
    set_checked(reopened.rvc_enabled, true);
    ACTIONS.lock().unwrap().push(RVC_ENABLED);
    unsafe { SendMessageW(reset_basic, BM_CLICK, None, None) };
    assert!(reopened.poll().is_none());
    let basic = reopened.read().unwrap();
    assert_eq!(basic.overlay_mode, OverlayMode::Always);
    assert_eq!(basic.startup_mode, None);
    assert_eq!(basic.microphone_id, "default://console");
    assert_eq!(basic.speaker_id, "default://console");
    assert_eq!(basic.cable_id, "default://console");
    assert!(basic.voice_input_id.is_empty() && basic.monitor_id.is_empty());
    assert!(!basic.cable_enabled && !basic.voice_input_enabled && !basic.monitoring);
    assert_eq!(basic.output_labels, crate::config::default_output_labels());
    assert!(basic.noise_suppression());
    assert!(!basic.rvc_enabled);
    assert_eq!(basic.rvc_parameters["pitch"], 17);
    assert_eq!(basic.switch_sounds[0], 7);

    // RVCタブのリセットは基本設定の「RVCを使用する」を保つ。
    set_checked(reopened.rvc_enabled, true);
    ACTIONS.lock().unwrap().push(RVC_ENABLED);
    assert!(reopened.poll().is_none());
    set_checked(reopened.noise, false);
    reopened.page(RVC_PAGE);
    let reset_rvc = unsafe { GetDlgItem(Some(reopened.hwnd), RESET_RVC as i32) }.unwrap();
    assert!(enabled(reset_rvc));
    unsafe { SendMessageW(reset_rvc, BM_CLICK, None, None) };
    assert!(reopened.poll().is_none());
    let rvc = reopened.read().unwrap();
    assert!(rvc.rvc_enabled);
    assert_eq!(rvc.rvc_parameters["pitch"], 10.0);
    assert_eq!(rvc.rvc_parameters["model"], "models/KahoA.pth");
    assert_eq!(rvc.rvc_parameters["f0_method"], "rmvpe");
    assert_eq!(rvc.rvc_parameters["n_cpu"], 4);
    assert!(!rvc.noise_suppression());
    assert_eq!(rvc.switch_sounds[0], 7);
    assert_eq!(reopened.base, custom);
}

#[test]
fn rvc_tooltips_cover_every_parameter_and_absolute_paths_roundtrip() {
    for p in schema().into_iter().filter(|p| p.id != NOISE_PARAMETER) {
        let help = super::parameters::parameter_help(&p);
        assert!(!help.is_empty(), "{}", p.id);
        assert!(help.contains(if p.restart_required {
            "再起動"
        } else {
            "反映"
        }));
        if p.kind == ParameterKind::File {
            let file = r"C:\任意のフォルダー\音声 # model.pth";
            assert_eq!(
                parse_parameter(&p, &format!("\"{file}\""), false).unwrap(),
                file
            );
        }
    }
}
