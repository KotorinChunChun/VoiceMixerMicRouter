use super::super::super::config::{default_hotkeys, Hotkey};
use super::super::drawing::{
    control_colors, control_text, draw_control, overlay_control_font, paint_buffered, rgb,
};
use super::super::{
    send_existing, Ui, CALLBACK, CLASS, CONTROL, EXIT, RELEASE_RVC, RETRY, SETTINGS, SHOW, TOGGLE,
};
use super::*;

#[test]
fn rvc_disabled_hides_inputs_and_meters_status_tracks_only_selected_input() {
    let keys = default_hotkeys().map(|k| k.key_label().into());
    let mut controls = overlay_controls(
        &keys,
        Mode::Direct,
        OverlayMode::Always,
        [true; 3],
        "RVC未設定",
        &crate::config::default_output_labels(),
        [false; 3],
    );
    configure_inputs(&mut controls, false, "running");
    assert!(!controls.iter().any(|c| matches!(c.action, RVC | SPK_RVC)));
    let row: Vec<_> = controls
        .iter()
        .filter(|c| c.rect.top == INPUT_TOP)
        .collect();
    assert_eq!(row.len(), 2);
    assert_eq!((row[0].rect.left, row[0].rect.right), (56, 250));
    assert_eq!((row[1].rect.left, row[1].rect.right), (254, 448));
    assert_eq!(control_at(&controls, 300, SETTINGS_TOP + 10), None);
    for (active, phase, expected) in [
        (false, "error", "無効"),
        (true, "starting", "ロード中"),
        (true, "error", "エラー"),
        (true, "running", ""),
    ] {
        assert_eq!(input_status(active, phase), expected);
    }
    let mut controls = overlay_controls(
        &keys,
        Mode::Rvc,
        OverlayMode::Always,
        [true; 3],
        "RVCノイズ抑制 ON",
        &crate::config::default_output_labels(),
        [false; 3],
    );
    configure_inputs(&mut controls, true, "starting");
    for (action, status) in [(DIRECT, "無効"), (RVC, "ロード中"), (SPK_RVC, "無効")] {
        assert_eq!(
            controls.iter().find(|c| c.action == action).unwrap().status,
            status
        );
    }
    for control in &controls {
        for area in TELEMETRY_RECTS {
            assert!(control.rect.bottom <= area.top || control.rect.top >= area.bottom);
        }
    }
}
#[test]
fn long_output_labels_shorten_without_hiding_key_or_on_off() {
    use super::super::drawing::fit_output_text as fit;
    assert_eq!(
        fit("会議[F9]ON", 30, |s| s.chars().count() as i32),
        "会議[F9]ON"
    );
    assert_eq!(
        fit("長い用途名OFF", 6, |s| s.chars().count() as i32),
        "長い…OFF"
    );
    assert_eq!(
        fit("長い用途名😀[F10]OFF", 12, |s| s.chars().count()
            as i32),
        "長い用…[F10]OFF"
    );
    assert_eq!(
        fit("😀😀😀[F11]ON", 9, |s| s.chars().count() as i32),
        "😀…[F11]ON"
    );
}

#[test]
fn renamed_output_buttons_keep_their_commands_keys_and_independent_states() {
    let controls = overlay_controls(
        &default_hotkeys().map(|key| key.key_label().to_owned()),
        Mode::Direct,
        OverlayMode::Always,
        [false, true, false],
        "RVCノイズ抑制 ON",
        &["会議", "ゲーム & 音楽", "確認"].map(String::from),
        [false; 3],
    );
    for (i, (action, expected, active)) in [
        (OUTPUT1, "会議[F9]OFF", false),
        (OUTPUT2, "ゲーム & 音楽[F10]ON", true),
        (MONITOR, "確認[F11]OFF", false),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(controls[i + 6].text, expected);
        assert_eq!(controls[i + 6].action, action);
        assert_eq!(controls[i + 6].active, active);
    }
}

#[test]
fn overlay_actions_match_drawn_cells_and_output_states() {
    let keys = default_hotkeys().map(|k| k.key_label().to_owned());
    let controls = overlay_controls(
        &keys,
        Mode::Direct,
        OverlayMode::Always,
        [true, false, true],
        "RVCノイズ抑制 ON",
        &crate::config::default_output_labels(),
        [false; 3],
    );
    assert_eq!(controls.len(), 9);
    assert_eq!(
        controls.iter().map(|c| c.action).collect::<Vec<_>>(),
        vec![OVERLAY, NOISE, DIRECT, RVC, SPK_RVC, MUTED, OUTPUT1, OUTPUT2, MONITOR]
    );
    for c in &controls {
        assert_eq!(
            control_at(&controls, (c.rect.left + c.rect.right) / 2, c.rect.top + 10),
            Some(c.action)
        );
        assert!(!c.text.contains("Alt"));
    }
    assert_eq!(control_at(&controls, 20, 12), None); // 見出しはドラッグ領域。
    assert_eq!(controls[6].text, "配信用[F9]ON");
    assert_eq!(controls[7].text, "音声入力用[F10]OFF");
    assert_eq!(controls[8].text, "モニタ用[F11]ON");
}

#[test]
fn input_selection_outputs_and_click_gaps_stay_independent() {
    let keys = default_hotkeys().map(|k| k.key_label().to_owned());
    for (mode, selected) in [
        (Mode::Direct, DIRECT),
        (Mode::Rvc, RVC),
        (Mode::SpeakerRvc, SPK_RVC),
        (Mode::Muted, MUTED),
    ] {
        for bits in 0..8 {
            let outputs = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
            let controls = overlay_controls(
                &keys,
                mode,
                OverlayMode::Always,
                outputs,
                "RVCノイズ抑制 ON",
                &crate::config::default_output_labels(),
                [false; 3],
            );
            let active: Vec<_> = controls
                .iter()
                .filter(|c| matches!(c.action, DIRECT | RVC | SPK_RVC | MUTED) && c.active)
                .map(|c| c.action)
                .collect();
            assert_eq!(active, vec![selected]);
            for (index, action) in [OUTPUT1, OUTPUT2, MONITOR].into_iter().enumerate() {
                assert_eq!(
                    controls.iter().find(|c| c.action == action).unwrap().active,
                    outputs[index]
                );
            }
            for y in [16, 40, 82] {
                for x in 18..56 {
                    assert_eq!(control_at(&controls, x, y), None);
                }
            }
            for (left, right, y) in [
                (151, 155, 40),
                (250, 254, 40),
                (349, 353, 40),
                (184, 188, 82),
                (316, 320, 82),
                (250, 254, 16),
            ] {
                for x in left..right {
                    assert_eq!(control_at(&controls, x, y), None);
                }
            }
            assert!(controls[0].active && controls[1].active);
        }
    }
    let off = overlay_controls(
        &keys,
        Mode::Direct,
        OverlayMode::Off,
        [false; 3],
        "RVCノイズ抑制 OFF",
        &crate::config::default_output_labels(),
        [false; 3],
    );
    assert!(!off[0].active && !off[1].active);
    let on = overlay_controls(
        &keys,
        Mode::Rvc,
        OverlayMode::Brief,
        [false; 3],
        "RVCノイズ抑制 ON",
        &crate::config::default_output_labels(),
        [false; 3],
    );
    assert!(on[0].active && on[1].active);
}

#[test]
fn countdown_and_cursor_follow_visibility_and_interaction_state() {
    for (ms, remaining) in [
        (0, Some(4)),
        (999, Some(4)),
        (1000, Some(3)),
        (1999, Some(3)),
        (2000, Some(2)),
        (3000, Some(1)),
        (3999, Some(1)),
        (4000, None),
        (5000, None),
    ] {
        let elapsed = Duration::from_millis(ms);
        assert_eq!(brief_remaining(OverlayMode::Brief, elapsed), remaining);
        assert_eq!(overlay_expired(OverlayMode::Brief, elapsed), ms >= 4000);
        assert_eq!(brief_remaining(OverlayMode::Always, elapsed), None);
        assert_eq!(brief_remaining(OverlayMode::Off, elapsed), None);
        assert!(!overlay_expired(OverlayMode::Always, elapsed));
        assert!(overlay_expired(OverlayMode::Off, elapsed));
    }
    assert_eq!(cursor_for_control(Some(DIRECT), false), IDC_HAND);
    assert_eq!(cursor_for_control(None, false), IDC_SIZEALL);
    assert_eq!(cursor_for_control(Some(DIRECT), true), IDC_SIZEALL);
    let actions = [
        DIRECT, RVC, SPK_RVC, MUTED, OUTPUT1, OUTPUT2, MONITOR, NOISE, OVERLAY,
    ];
    for (locked, escape, expected) in [
        (false, false, actions.to_vec()),
        (true, false, vec![]),
        (true, true, vec![DIRECT, MUTED]),
    ] {
        let allowed: Vec<_> = actions
            .into_iter()
            .filter(|action| overlay_action_allowed_with(*action, locked, escape))
            .collect();
        assert_eq!(allowed, expected);
        for action in actions {
            let control =
                Some(action).filter(|action| overlay_action_allowed_with(*action, locked, escape));
            assert_eq!(
                cursor_for_control(control, false),
                if expected.contains(&action) {
                    IDC_HAND
                } else {
                    IDC_SIZEALL
                }
            );
            assert_eq!(cursor_for_control(control, true), IDC_SIZEALL);
        }
    }
}

#[test]
fn buttons_render_on_off_pixels_and_fit_long_keys_at_twelve_pixels() {
    // 画面や実カーソルに触れず、描画に使用する関数をメモリーDCへ出力して確認する。
    unsafe {
        let dc = CreateCompatibleDC(None);
        assert!(!dc.0.is_null());
        let mut pixels = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            Some(dc),
            &BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: OVERLAY_WIDTH,
                    biHeight: -142,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            DIB_RGB_COLORS,
            &mut pixels,
            None,
            0,
        )
        .unwrap();
        let previous_bitmap = SelectObject(dc, bitmap.into());
        let font = overlay_control_font();
        let previous_font = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        for key in ["F24", "なし"] {
            let keys = std::array::from_fn(|_| key.to_string());
            let controls = overlay_controls(
                &keys,
                Mode::Direct,
                OverlayMode::Brief,
                [false; 3],
                "RVCノイズ抑制 ON",
                &[
                    "音声入力用".into(),
                    "音声入力用".into(),
                    "音声入力用".into(),
                ],
                [false; 3],
            );
            for control in controls {
                let text: Vec<u16> = control_text(&control, 4).encode_utf16().collect();
                let mut size = windows::Win32::Foundation::SIZE::default();
                assert!(GetTextExtentPoint32W(dc, &text, &mut size).as_bool());
                assert!(
                    size.cx <= control.rect.right - control.rect.left - 4,
                    "{}: {}px > {}px",
                    control.text,
                    size.cx,
                    control.rect.right - control.rect.left - 4
                );
                assert!(size.cy <= control.rect.bottom - control.rect.top - 2);
                let mut on = control.clone();
                on.active = true;
                draw_control(dc, &on, 4);
                let on_pixel = GetPixel(dc, on.rect.left + 2, on.rect.top + 2);
                assert_eq!(on_pixel, control_colors(&on).0);
                assert_eq!(
                    GetPixel(dc, on.rect.left, on.rect.top),
                    control_colors(&on).1
                );
                let mut off = on;
                off.active = false;
                draw_control(dc, &off, 4);
                let off_pixel = GetPixel(dc, off.rect.left + 2, off.rect.top + 2);
                assert_eq!(off_pixel, control_colors(&off).0);
                assert_ne!(on_pixel, off_pixel);
            }
        }
        SelectObject(dc, previous_font);
        SelectObject(dc, previous_bitmap);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
    }
}

#[test]
fn buffered_paint_keeps_incomplete_frame_hidden_and_limits_transfer() {
    unsafe {
        let dc = CreateCompatibleDC(None);
        assert!(!dc.0.is_null());
        let mut pixels = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            Some(dc),
            &BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: OVERLAY_WIDTH,
                    biHeight: -142,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            DIB_RGB_COLORS,
            &mut pixels,
            None,
            0,
        )
        .unwrap();
        let previous = SelectObject(dc, bitmap.into());
        let bounds = RECT {
            left: 0,
            top: 0,
            right: OVERLAY_WIDTH,
            bottom: 142,
        };
        let fill = |target, area: &RECT, color| {
            let brush = CreateSolidBrush(color);
            FillRect(target, area, brush);
            let _ = DeleteObject(brush.into());
        };
        let original = rgb(190, 20, 180);
        let background = rgb(28, 32, 40);
        let completed = rgb(60, 180, 70);
        fill(dc, &bounds, original);
        paint_buffered(dc, bounds, TELEMETRY_RECT, |buffer| {
            fill(buffer, &bounds, background);
            // 背景を描いた途中で、表示先が消されていない。
            assert_eq!(GetPixel(dc, 300, 58), original);
            fill(buffer, &TELEMETRY_RECT, completed);
            assert_eq!(GetPixel(dc, 300, 58), original);
        });
        assert_eq!(GetPixel(dc, 300, 58), completed);
        // 文字・ボタンなど、更新範囲外の内容を転送で消さない。
        assert_eq!(GetPixel(dc, 70, 40), original);
        assert_eq!(GetPixel(dc, 55, 58), original);
        assert_eq!(GetPixel(dc, 448, 58), original);
        assert_eq!(GetPixel(dc, 300, 40), original);
        // メーター消失時も更新領域を背景で消し、残像を残さない。
        paint_buffered(dc, bounds, TELEMETRY_RECT, |buffer| {
            fill(buffer, &bounds, background)
        });
        assert_eq!(GetPixel(dc, 300, 58), background);
        assert_eq!(GetPixel(dc, 70, 40), original);
        SelectObject(dc, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
    }
}

#[test]
#[ignore = "通常ユーザーのWindows通知領域を使用する実機GUI試験"]
fn native_commands_menu_icons_and_nonactivating_overlay() {
    // 実音声は起動せず、対象ウィンドウへのメッセージだけで操作する。
    if unsafe { FindWindowW(PCWSTR(wide(CLASS).as_ptr()), PCWSTR::null()) }.is_ok() {
        panic!("GUI試験はマイク常駐を停止してから実行してください");
    }
    let keys = std::array::from_fn(|_| Hotkey {
        handling: crate::config::KeyHandling::Coexist,
        enabled: false,
        shift: false,
        ctrl: false,
        win: false,
        alt: false,
        key: "V".into(),
    });
    let mut ui = Ui::new(Mode::Direct, &keys, OverlayMode::Brief, None, false, false).unwrap();
    ui.update(Mode::Direct, "running", "MIC").unwrap();
    assert_eq!(ui.overlay.state.paint.lock().unwrap().0, "MIC 出力中");
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("数秒表示"));
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("モニタ用OFF"));
    // Win32が実際に描いた画素を確認する。元配列だけの比較では上下反転を見逃す。
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, 32, 32);
        let previous = SelectObject(dc, bitmap.into());
        for (handle, rows) in [
            (ui.icons[0], [17u8, 27, 21, 21, 17, 17, 17]),
            (ui.icons[1], [30u8, 17, 17, 30, 20, 18, 17]),
            (ui.icons[5], [15u8, 16, 16, 14, 1, 1, 30]),
        ] {
            DrawIconEx(dc, 0, 0, handle, 32, 32, 0, None, DI_NORMAL).unwrap();
            for (y, row) in rows.into_iter().enumerate() {
                for x in 0..5 {
                    let white = GetPixel(dc, 8 + x * 4, 4 + y as i32 * 4).0 == 0xffffff;
                    assert_eq!(white, row & (1 << (4 - x)) != 0, "glyph {x},{y}");
                }
            }
        }
        SelectObject(dc, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
    }
    let direct_icon = ui.data.hIcon;
    unsafe {
        assert_ne!(
            GetWindowLongW(ui.overlay.hwnd, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0,
            0
        );
        let menu = ui.menu().unwrap();
        assert_ne!(
            GetMenuState(menu, DIRECT as u32, MF_BYCOMMAND) & MF_CHECKED.0,
            0
        );
        DestroyMenu(menu).unwrap();
        PostMessageW(Some(ui.hwnd), CONTROL, WPARAM(TOGGLE), LPARAM(0)).unwrap();
    }
    assert_eq!(ui.poll().unwrap(), vec![TOGGLE]);
    unsafe {
        PostMessageW(
            Some(ui.hwnd),
            CALLBACK,
            WPARAM(1),
            LPARAM(WM_LBUTTONDBLCLK as isize),
        )
        .unwrap();
    }
    assert_eq!(ui.poll().unwrap(), vec![SETTINGS]);
    ui.interaction_locked = true;
    unsafe {
        let menu = ui.menu().unwrap();
        for id in [
            DIRECT,
            RVC,
            SPK_RVC,
            RETRY,
            RELEASE_RVC,
            MUTED,
            SETTINGS,
            OVERLAY,
            MONITOR,
            NOISE,
            20,
            21,
            22,
            23,
            24,
            EXIT,
        ] {
            let state = GetMenuState(menu, id as u32, MF_BYCOMMAND);
            assert_ne!(state, u32::MAX);
            assert_eq!(state & MF_GRAYED.0 != 0, id != SETTINGS, "menu {id}");
        }
        DestroyMenu(menu).unwrap();
    }
    ui.interaction_locked = false;
    ui.set_display(OverlayMode::Always, true, true).unwrap();
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("RVCノイズ抑制 ON"));
    ui.rvc_enabled = false;
    ui.update(Mode::Direct, "running", "MIC").unwrap();
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("RVC未設定"));
    assert_eq!(
        ui.overlay
            .state
            .controls
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.action == NOISE)
            .unwrap()
            .text,
        "RVC未設定"
    );
    ui.rvc_enabled = true;
    ui.update(Mode::Rvc, "running", "RVC").unwrap();
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("RVCノイズ抑制 ON"));
    unsafe {
        let menu = ui.menu().unwrap();
        assert_ne!(
            GetMenuState(menu, NOISE as u32, MF_BYCOMMAND) & MF_CHECKED.0,
            0
        );
        DestroyMenu(menu).unwrap();
    }
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("常時表示"));
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("モニタ用ON"));
    ui.overlay.shown = Instant::now() - Duration::from_secs(5);
    ui.poll().unwrap();
    assert!(unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    ui.set_display(OverlayMode::Off, false, false).unwrap();
    assert!(!unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    for phase in ["starting", "error", "running"] {
        ui.update(Mode::Rvc, phase, "非表示試験").unwrap();
        ui.show().unwrap();
        assert!(!unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    }
    ui.set_display(OverlayMode::Brief, false, false).unwrap();
    let mut assigned = keys.clone();
    for (i, k) in assigned.iter_mut().enumerate() {
        k.enabled = true;
        k.ctrl = true;
        k.alt = true;
        k.shift = true;
        k.key = format!("F{}", 1 + i);
    }
    ui.set_hotkeys(&assigned).unwrap();
    for action in crate::actions::HOTKEY_COMMANDS {
        unsafe {
            PostMessageW(Some(ui.hwnd), CONTROL, WPARAM(action), LPARAM(0)).unwrap();
        }
        assert_eq!(ui.poll().unwrap(), vec![action]);
    }
    assigned.swap(0, 2);
    ui.set_hotkeys(&assigned).unwrap();
    // 本ツールの割当と同じキーを別アプリが予約できる。重複は本ツール内だけで拒否する。
    unsafe {
        RegisterHotKey(None, 9000, MOD_CONTROL | MOD_ALT | MOD_SHIFT, 0x70).unwrap();
    }
    ui.set_hotkeys(&assigned).unwrap();
    let mut conflicting = assigned.clone();
    conflicting[0] = conflicting[1].clone();
    let labels = ui.shortcut_labels.clone();
    assert!(ui.set_hotkeys(&conflicting).is_err());
    assert_eq!(ui.shortcut_labels, labels);
    unsafe {
        UnregisterHotKey(None, 9000).unwrap();
    }
    // なしは有効フラグが残っていても解除でき、修飾キーなしの実キーは登録できる。
    assigned[2].key = "None".into();
    assigned[3].key = "Z".into();
    assigned[3].shift = false;
    assigned[3].ctrl = false;
    assigned[3].alt = false;
    ui.set_hotkeys(&assigned).unwrap();
    assert_eq!(ui.shortcut_keys[2], "なし");
    assert_eq!(ui.shortcut_labels[3], "Z");
    ui.set_hotkeys(&default_hotkeys()).unwrap();
    ui.update(Mode::Direct, "running", "MIC").unwrap();
    assert_eq!(
        ui.overlay.state.paint.lock().unwrap().2,
        "MIC[F5]  MIC-RVC[F6]  SPK-RVC[F7]  停止[F8]"
    );
    assert_eq!(
        ui.overlay.state.paint.lock().unwrap().1,
        "表示[F12]数秒表示  RVCノイズ抑制 OFF / 配信用[F9]ON 音声入力用[F10]OFF モニタ用[F11]OFF"
    );
    // 実カーソルを動かさず、各メッセージの座標だけで押したセルを選ぶ。
    // GetCursorPos で押下位置を読み直す実装では、この9セルを区別できない。
    let mouse_position =
        |x: i32, y: i32| LPARAM((((y as u16 as u32) << 16) | x as u16 as u32) as isize);
    let controls = ui.overlay.state.controls.lock().unwrap().clone();
    for control in controls {
        let x = (control.rect.left + control.rect.right) / 2;
        let y = control.rect.top + 10;
        unsafe {
            SendMessageW(
                ui.overlay.hwnd,
                WM_LBUTTONDOWN,
                None,
                Some(mouse_position(x, y)),
            );
            assert_eq!(
                ui.overlay.state.drag.lock().unwrap().unwrap().3,
                Some(control.action)
            );
            SendMessageW(
                ui.overlay.hwnd,
                WM_LBUTTONUP,
                None,
                Some(mouse_position(x, y)),
            );
        }
        assert_eq!(ui.poll().unwrap(), vec![control.action]);
    }
    // 移動メッセージなしで離しても、別セルや4px以上の移動をクリックにしない。
    for (end_x, expected) in [(140, None), (74, None), (73, Some(DIRECT))] {
        unsafe {
            SendMessageW(
                ui.overlay.hwnd,
                WM_LBUTTONDOWN,
                None,
                Some(mouse_position(70, 40)),
            );
            SendMessageW(
                ui.overlay.hwnd,
                WM_LBUTTONUP,
                None,
                Some(mouse_position(end_x, 40)),
            );
        }
        assert_eq!(ui.poll().unwrap(), expected.into_iter().collect::<Vec<_>>());
    }
    // ボタンから隙間へ1px出ただけでも、押下と解放の対象が違えば操作しない。
    unsafe {
        SendMessageW(
            ui.overlay.hwnd,
            WM_LBUTTONDOWN,
            None,
            Some(mouse_position(150, 40)),
        );
        SendMessageW(
            ui.overlay.hwnd,
            WM_LBUTTONUP,
            None,
            Some(mouse_position(151, 40)),
        );
    }
    assert!(ui.poll().unwrap().is_empty());
    unsafe {
        SendMessageW(
            ui.overlay.hwnd,
            WM_LBUTTONDOWN,
            None,
            Some(mouse_position(70, 40)),
        );
        SendMessageW(ui.overlay.hwnd, WM_CANCELMODE, None, None);
        SendMessageW(
            ui.overlay.hwnd,
            WM_LBUTTONUP,
            None,
            Some(mouse_position(70, 40)),
        );
    }
    assert!(ui.poll().unwrap().is_empty());
    assert_ne!(unsafe { GetCapture() }, ui.overlay.hwnd);
    // 選択状態を変えず、操作できる場所だけ手形カーソルにする。
    let old_cursor = unsafe { GetCursor() };
    unsafe {
        let hand = LoadCursorW(None, IDC_HAND).unwrap();
        let move_cursor = LoadCursorW(None, IDC_SIZEALL).unwrap();
        ui.overlay
            .state
            .set_overlay_cursor_at(POINT { x: 70, y: 40 });
        assert_eq!(GetCursor(), hand);
        ui.overlay
            .state
            .set_overlay_cursor_at(POINT { x: 30, y: 40 });
        assert_eq!(GetCursor(), move_cursor);
        SendMessageW(
            ui.overlay.hwnd,
            WM_LBUTTONDOWN,
            None,
            Some(mouse_position(70, 40)),
        );
        assert_eq!(GetCursor(), move_cursor);
        SendMessageW(ui.overlay.hwnd, WM_CANCELMODE, None, None);
    }
    ui.interaction_locked = true;
    ui.poll().unwrap();
    let selection: Vec<_> = ui
        .overlay
        .state
        .controls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.active)
        .collect();
    let controls = ui.overlay.state.controls.lock().unwrap().clone();
    // 設定画面中は全操作禁止。画面のない適用中だけMIC・送信停止へ戻れる。
    for (escape, allowed) in [(false, vec![]), (true, vec![DIRECT, MUTED])] {
        ui.allow_mode_escape = escape;
        ui.poll().unwrap();
        for control in &controls {
            let x = (control.rect.left + control.rect.right) / 2;
            let y = control.rect.top + 10;
            let expected = allowed.contains(&control.action);
            unsafe {
                ui.overlay.state.set_overlay_cursor_at(POINT { x, y });
                assert_eq!(
                    GetCursor(),
                    LoadCursorW(None, if expected { IDC_HAND } else { IDC_SIZEALL }).unwrap()
                );
                SendMessageW(
                    ui.overlay.hwnd,
                    WM_LBUTTONDOWN,
                    None,
                    Some(mouse_position(x, y)),
                );
                assert_eq!(
                    ui.overlay.state.drag.lock().unwrap().unwrap().3,
                    expected.then_some(control.action)
                );
                SendMessageW(
                    ui.overlay.hwnd,
                    WM_LBUTTONUP,
                    None,
                    Some(mouse_position(x, y)),
                );
            }
            assert_eq!(
                ui.poll().unwrap(),
                if expected {
                    vec![control.action]
                } else {
                    vec![]
                }
            );
        }
    }
    // 押した後で設定画面が開かれた場合も、解放時の判定で操作を止める。
    unsafe {
        SendMessageW(
            ui.overlay.hwnd,
            WM_LBUTTONDOWN,
            None,
            Some(mouse_position(70, 40)),
        );
    }
    ui.allow_mode_escape = false;
    ui.poll().unwrap();
    unsafe {
        SendMessageW(
            ui.overlay.hwnd,
            WM_LBUTTONUP,
            None,
            Some(mouse_position(70, 40)),
        );
    }
    assert!(ui.poll().unwrap().is_empty());
    assert_eq!(
        ui.overlay
            .state
            .controls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.active)
            .collect::<Vec<_>>(),
        selection
    );
    ui.interaction_locked = false;
    ui.poll().unwrap();
    unsafe {
        SetCursor(Some(old_cursor));
    }
    // カウントダウン描画と同じ状態の定期報告は、通知・期限を書き換えない。
    ui.show().unwrap();
    assert_eq!(ui.overlay.state.countdown.load(Ordering::Relaxed), 4);
    ui.overlay.shown = Instant::now() - Duration::from_millis(1500);
    let shown = ui.overlay.shown;
    let tip = ui.data.szTip;
    ui.overlay.refresh_countdown(Instant::now());
    assert_eq!(ui.overlay.state.countdown.load(Ordering::Relaxed), 3);
    ui.update(Mode::Direct, "running", "定期報告").unwrap();
    ui.update_telemetry(None, 1);
    ui.poll().unwrap();
    assert_eq!(ui.overlay.shown, shown);
    assert_eq!(ui.data.szTip, tip);
    assert_eq!(ui.overlay.state.countdown.load(Ordering::Relaxed), 3);
    // 同じ状態の報告は再描画せず、メーター更新も上部に限定する。
    unsafe {
        let _ = ValidateRect(Some(ui.overlay.hwnd), None);
    }
    ui.update(Mode::Direct, "running", "同一状態の再通知")
        .unwrap();
    assert!(!unsafe { GetUpdateRect(ui.overlay.hwnd, None, false) }.as_bool());
    ui.overlay.meter_painted = Instant::now() - telemetry::INTERVAL;
    ui.update_telemetry(None, 1);
    let mut dirty = RECT::default();
    assert!(unsafe { GetUpdateRect(ui.overlay.hwnd, Some(&mut dirty), false) }.as_bool());
    assert_eq!(dirty, TELEMETRY_RECT);
    assert_eq!(
        unsafe { SendMessageW(ui.overlay.hwnd, WM_ERASEBKGND, None, None) },
        LRESULT(1)
    );
    unsafe {
        let _ = UpdateWindow(ui.overlay.hwnd);
    }
    ui.overlay.shown = Instant::now() - BRIEF_DURATION;
    ui.poll().unwrap();
    assert_eq!(ui.overlay.state.countdown.load(Ordering::Relaxed), 0);
    assert!(!unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    ui.update(Mode::Direct, "running", "定期報告").unwrap();
    assert!(!unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    ui.show().unwrap();
    assert_eq!(ui.overlay.state.countdown.load(Ordering::Relaxed), 4);
    // フォーカスを奪わずドラッグし、モード変更・Off/Onでも座標を保持する。
    let foreground = unsafe { GetForegroundWindow() };
    let mut before_drag = RECT::default();
    unsafe {
        GetWindowRect(ui.overlay.hwnd, &mut before_drag).unwrap();
        SendMessageW(ui.overlay.hwnd, WM_LBUTTONDOWN, None, None);
        let (start, _, _, _) = ui.overlay.state.drag.lock().unwrap().unwrap();
        ui.overlay.state.drag_to(
            ui.overlay.hwnd,
            POINT {
                x: start.x - 100,
                y: start.y + 90,
            },
        );
    }
    ui.overlay.shown = Instant::now() - Duration::from_secs(5);
    ui.poll().unwrap();
    assert!(unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    unsafe {
        SendMessageW(ui.overlay.hwnd, WM_LBUTTONUP, None, None);
    }
    assert!(ui.overlay.state.drag.lock().unwrap().is_none());
    ui.set_display(OverlayMode::Off, false, false).unwrap();
    ui.set_display(OverlayMode::Brief, false, false).unwrap();
    ui.update(Mode::Muted, "running", "停止").unwrap();
    let mut moved = RECT::default();
    unsafe {
        GetWindowRect(ui.overlay.hwnd, &mut moved).unwrap();
    }
    assert_eq!(
        (moved.left, moved.top),
        (before_drag.left - 100, before_drag.top + 90)
    );
    assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    unsafe {
        SendMessageW(ui.overlay.hwnd, WM_LBUTTONDOWN, None, None);
    }
    ui.set_display(OverlayMode::Off, false, false).unwrap();
    assert!(ui.overlay.state.drag.lock().unwrap().is_none());
    assert_ne!(unsafe { GetCapture() }, ui.overlay.hwnd);
    ui.set_display(OverlayMode::Brief, false, false).unwrap();
    assert!(send_existing(RVC).unwrap());
    assert_eq!(ui.poll().unwrap(), vec![RVC]);
    ui.update(Mode::Rvc, "running", "RVC").unwrap();
    assert_ne!(ui.data.hIcon, direct_icon);
    assert!(ui.overlay.state.paint.lock().unwrap().0.contains("RVC"));
    ui.update(Mode::SpeakerRvc, "running", "SPK-RVC").unwrap();
    assert_eq!(ui.data.hIcon, ui.icons[5]);
    assert_eq!(ui.overlay.state.paint.lock().unwrap().0, "SPK-RVC 出力中");
    assert!(send_existing(SPK_RVC).unwrap());
    assert_eq!(ui.poll().unwrap(), vec![SPK_RVC]);
    ui.update(Mode::Rvc, "running", "MIC-RVC").unwrap();
    ui.overlay.shown = Instant::now() - Duration::from_secs(5);
    ui.poll().unwrap();
    assert!(!unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    ui.update(Mode::Rvc, "error", "テストエラー").unwrap();
    assert!(unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    let error_icon = ui.data.hIcon;
    assert_ne!(error_icon, direct_icon);
    assert_ne!(error_icon, ui.icons[1]);
    ui.overlay.shown = Instant::now() - Duration::from_secs(5);
    ui.poll().unwrap();
    assert!(!unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    // 定期状態更新で通知期限を延長しない。モニター変更は同じモードでも通知する。
    ui.update(Mode::Rvc, "error", "テストエラー").unwrap();
    assert!(!unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    ui.set_display(OverlayMode::Brief, true, false).unwrap();
    assert!(unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("モニタ用[F11]ON"));
    // 測定値更新が通知期限・フォーカス・位置を変えず、古い世代を表示しない。
    ui.update(Mode::Rvc, "running", "RVC").unwrap();
    ui.set_display(OverlayMode::Brief, false, false).unwrap();
    let mut before = RECT::default();
    unsafe {
        GetWindowRect(ui.overlay.hwnd, &mut before).unwrap();
    }
    ui.overlay.shown = Instant::now() - Duration::from_secs(5);
    let expiry = ui.overlay.shown;
    let foreground = unsafe { GetForegroundWindow() };
    let mut snapshot = Snapshot {
        generation: 1,
        mode: Mode::Rvc,
        at: Instant::now(),
        samples: vec![],
        latency_ms: Some(388.0),
        outputs_enabled: [true; 3],
    };
    ui.overlay.meter_painted = Instant::now() - telemetry::INTERVAL;
    ui.update_telemetry(Some(snapshot.clone()), 1);
    assert_eq!(ui.overlay.shown, expiry);
    assert!(ui.overlay.state.meters.lock().unwrap().is_some());
    ui.poll().unwrap();
    assert!(!unsafe { IsWindowVisible(ui.overlay.hwnd) }.as_bool());
    assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    let mut after = RECT::default();
    unsafe {
        GetWindowRect(ui.overlay.hwnd, &mut after).unwrap();
    }
    assert_eq!(before, after);
    snapshot.generation = 2;
    ui.overlay.meter_painted = Instant::now() - telemetry::INTERVAL;
    ui.update_telemetry(Some(snapshot), 1);
    assert!(ui.overlay.state.meters.lock().unwrap().is_none());
    ui.update(Mode::Muted, "running", "送信停止").unwrap();
    assert_eq!(ui.overlay.state.paint.lock().unwrap().0, "送信停止");
    assert_eq!(ui.data.hIcon, ui.icons[4]);
    unsafe {
        let menu = ui.menu().unwrap();
        assert_ne!(
            GetMenuState(menu, MUTED as u32, MF_BYCOMMAND) & MF_CHECKED.0,
            0
        );
        DestroyMenu(menu).unwrap();
    }
    ui.output_labels = ["会議", "ゲーム & 音楽", "確認"].map(String::from);
    ui.update(Mode::Muted, "running", "送信停止").unwrap();
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("会議[F9]"));
    assert_eq!(
        ui.overlay.state.controls.lock().unwrap()[7].text,
        "ゲーム & 音楽[F10]OFF"
    );
    unsafe {
        let menu = ui.menu().unwrap();
        for (action, label) in [
            (OUTPUT1, "出力1（会議）"),
            (OUTPUT2, "出力2（ゲーム && 音楽）"),
            (MONITOR, "出力3（確認）"),
        ] {
            let mut value = [0u16; 128];
            let length = GetMenuStringW(menu, action as u32, Some(&mut value), MF_BYCOMMAND);
            assert!(String::from_utf16_lossy(&value[..length as usize]).starts_with(label));
        }
        DestroyMenu(menu).unwrap();
    }
    // もともとONだった出力も、P2T押下と解放で表示内容が変わる。
    ui.output_holds = [false, true, false];
    ui.cable_enabled = false;
    ui.voice_input_enabled = true;
    ui.monitoring = false;
    ui.update(Mode::Direct, "running", "MIC").unwrap();
    assert!(ui.overlay.state.controls.lock().unwrap()[7].held);
    assert!(ui
        .overlay
        .state
        .paint
        .lock()
        .unwrap()
        .1
        .contains("音楽[F10]P2T"));
    ui.output_holds = [false; 3];
    ui.update(Mode::Direct, "running", "MIC").unwrap();
    assert!(!ui.overlay.state.controls.lock().unwrap()[7].held);
    assert!(ui.overlay.state.controls.lock().unwrap()[7]
        .text
        .ends_with("ON"));
    drop(ui);
    assert!(!send_existing(SHOW).unwrap());
}

#[test]
fn equal_buttons_share_row_edges_and_keep_four_pixel_gaps() {
    let keys = default_hotkeys().map(|k| k.key_label().into());
    let controls = overlay_controls(
        &keys,
        Mode::Direct,
        OverlayMode::Always,
        [true; 3],
        "RVCノイズ抑制 ON",
        &crate::config::default_output_labels(),
        [false; 3],
    );
    for (top, count) in [(SETTINGS_TOP, 2), (INPUT_TOP, 4), (OUTPUT_TOP, 3)] {
        let row: Vec<_> = controls.iter().filter(|c| c.rect.top == top).collect();
        assert_eq!(row.len(), count);
        assert_eq!(row[0].rect.left, 56);
        assert_eq!(row[count - 1].rect.right, 448);
        let width = row[0].rect.right - row[0].rect.left;
        for c in &row {
            assert_eq!(c.rect.right - c.rect.left, width);
        }
        for pair in row.windows(2) {
            assert_eq!(pair[1].rect.left - pair[0].rect.right, 4);
        }
    }
}

#[test]
fn ptt_button_has_distinct_caption_color_and_only_pressed_outputs_light_up() {
    let keys = default_hotkeys().map(|k| k.key_label().into());
    let labels = [
        "音声入力用".into(),
        "音声入力用".into(),
        "音声入力用".into(),
    ];
    let normal = overlay_controls(
        &keys,
        Mode::Direct,
        OverlayMode::Always,
        [true; 3],
        "RVCノイズ抑制 ON",
        &labels,
        [false; 3],
    );
    for mask in 1..8 {
        let holds = [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0];
        let buttons = overlay_controls(
            &keys,
            Mode::Direct,
            OverlayMode::Always,
            holds,
            "RVCノイズ抑制 ON",
            &labels,
            holds,
        );
        for i in 0..3 {
            let b = &buttons[6 + i];
            assert_eq!(b.active, holds[i]);
            assert_eq!(b.held, holds[i]);
            if holds[i] {
                assert!(b.text.ends_with("P2T"));
                assert_ne!(control_colors(b), control_colors(&normal[6 + i]));
            } else {
                assert!(b.text.ends_with("OFF"));
            }
        }
    }
    assert_eq!(
        super::super::drawing::fit_output_text("長い用途名P2T", 6, |s| s.chars().count()
            as i32),
        "長い…P2T"
    );
    // 実際の描画フォントでP2T表示も全角5文字を省略しない。
    unsafe {
        let dc = CreateCompatibleDC(None);
        let font = overlay_control_font();
        let previous = SelectObject(dc, font.into());
        for key in ["F24", "なし"] {
            let buttons = overlay_controls(
                &std::array::from_fn(|_| key.into()),
                Mode::Direct,
                OverlayMode::Always,
                [true; 3],
                "RVCノイズ抑制 ON",
                &labels,
                [true; 3],
            );
            for b in &buttons[6..] {
                let mut size = windows::Win32::Foundation::SIZE::default();
                assert!(GetTextExtentPoint32W(
                    dc,
                    &b.text.encode_utf16().collect::<Vec<_>>(),
                    &mut size
                )
                .as_bool());
                assert!(
                    size.cx <= b.rect.right - b.rect.left - 4,
                    "{}: {}",
                    b.text,
                    size.cx
                );
            }
        }
        SelectObject(dc, previous);
        let _ = DeleteObject(font.into());
        let _ = DeleteDC(dc);
    }
}
