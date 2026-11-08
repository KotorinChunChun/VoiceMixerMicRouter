//! 基本設定とRVC設定の構築・読み取り・操作。
use super::super::{
    config::{
        default_hotkeys, device_choices,
        initial_rvc_parameters, is_default_device, DeviceSelection, Hotkey, KeyHandling, Mode,
        ModifierSide, OverlayMode, Settings, HOTKEY_COUNT, HOTKEY_LABELS, NOISE_PARAMETER,
        UNSELECTED,
    },
    ui,
};
use super::{
    controls::{checked, select, select_label, selection, set_checked, set_text, text, wide},
    display_order,
    parameters::{
        parameter_help, parameter_label, parameter_text, parameter_value_changed, parse_parameter,
    },
    Action, DeviceControl, ParameterControl, SettingsWindow, ACTIONS, CANCEL, CLASS, CLIENT_HEIGHT,
    CLIENT_WIDTH, HOTKEY_BASE, RESET_BASIC, RESET_KEYS, RESET_RVC, RESET_SOUNDS, RVC_ENABLED,
    RVC_PAGE, SAVE, SHOW_UNSELECTED, TAB, TAB_HEIGHT, TAB_TITLES, TAB_WIDTH, WINDOW,
};
use audio_mixer_backend::{
    plugins::{
        manifest::{ParameterDescriptor, ParameterKind},
        package::load_package_manifest,
    },
    windows::devices::{list_devices, AudioDeviceInfo, DeviceDirection},
};
use std::sync::atomic::Ordering;
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::*, Input::KeyboardAndMouse::EnableWindow, Shell::SetWindowSubclass,
            WindowsAndMessaging::*,
        },
    },
};

impl SettingsWindow {
    pub fn new(settings: &Settings, mode: Mode) -> Result<Self, String> {
        let inputs = list_devices(DeviceDirection::Capture).map_err(|e| e.to_string())?;
        let outputs = list_devices(DeviceDirection::Render).map_err(|e| e.to_string())?;
        let schema = load_package_manifest(settings.plugin_directory.join("rvc-cuda/plugin.json"))
            .map(|m| m.descriptor.parameters)
            .map_err(|e| e.to_string())?;
        Self::build(settings, &inputs, &outputs, schema, mode)
    }
    pub(super) fn build(
        settings: &Settings,
        inputs: &[AudioDeviceInfo],
        outputs: &[AudioDeviceInfo],
        schema: Vec<ParameterDescriptor>,
        mode: Mode,
    ) -> Result<Self, String> {
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            if RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(super::proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(wide(CLASS).as_ptr()),
                hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
                hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as usize as *mut _),
                ..Default::default()
            }) == 0
            {
                return Err(windows::core::Error::from_thread().to_string());
            }
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: CLIENT_WIDTH,
                bottom: CLIENT_HEIGHT,
            };
            AdjustWindowRectEx(&mut rect, style, false, WINDOW_EX_STYLE(0))
                .map_err(|e| e.to_string())?;
            let hwnd = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                PCWSTR(wide(CLASS).as_ptr()),
                PCWSTR(wide(&crate::window_title::WindowTitle::create("設定")).as_ptr()),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            let font = CreateFontW(
                -16,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                PCWSTR(wide("Yu Gothic UI").as_ptr()),
            );
            let mut s = Self {
                hwnd,
                font,
                link_font: {
                    let mut description = LOGFONTW::default();
                    GetObjectW(
                        font.into(),
                        std::mem::size_of::<LOGFONTW>() as i32,
                        Some((&mut description as *mut LOGFONTW).cast()),
                    );
                    description.lfUnderline = 1;
                    CreateFontIndirectW(&description)
                },
                tips: None,
                icon: HICON::default(),
                base: settings.clone(),
                pages: std::array::from_fn(|_| Vec::new()),
                tab: HWND::default(),
                active_page: std::cell::Cell::new(0),
                keys: vec![[HWND::default(); 7]; HOTKEY_COUNT],
                devices: Vec::new(),
                device_rows: std::array::from_fn(|_| Vec::new()),
                show_unselected: false,
                expand_devices: HWND::default(),
                parameters: Vec::new(),
                sounds: [HWND::default(); 7],
                sound_device: None,
                overlay: HWND::default(),
                outputs: [HWND::default(); 3],
                output_labels: [HWND::default(); 3],
                shortcut_titles: vec![HWND::default(); HOTKEY_COUNT],
                noise: HWND::default(),
                rvc_enabled: HWND::default(),
                startup: HWND::default(),
                startup_modes: Vec::new(),
                save: HWND::default(),
                cancel: HWND::default(),
                message: HWND::default(),
                busy: false,
                rvc_reset: false,
            };
            s.icon = ui::mode_icon(mode)?;
            for size in [ICON_SMALL, ICON_BIG] {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(size as usize)),
                    Some(LPARAM(s.icon.0 as isize)),
                );
            }
            if !InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_TAB_CLASSES,
            })
            .as_bool()
            {
                return Err("設定タブを初期化できません".into());
            }
            s.tips = Some(super::tooltip::Tooltips::new(hwnd, font)?);
            s.tab = s.control(
                "SysTabControl32",
                "",
                TCS_FIXEDWIDTH | TCS_OWNERDRAWFIXED | WS_CLIPSIBLINGS.0,
                TAB,
                10,
                16,
                CLIENT_WIDTH - 20,
                600,
                None,
            )?;
            if !SetWindowSubclass(s.tab, Some(super::tab_proc), 0, hwnd.0 as usize).as_bool() {
                return Err("設定タブの操作を初期化できません".into());
            }
            SendMessageW(
                s.tab,
                TCM_SETITEMSIZE,
                None,
                Some(LPARAM(((TAB_HEIGHT << 16) | TAB_WIDTH) as isize)),
            );
            for (index, title) in TAB_TITLES.iter().enumerate() {
                let mut label = wide(title);
                let item = TCITEMW {
                    mask: TCIF_TEXT,
                    pszText: PWSTR(label.as_mut_ptr()),
                    ..Default::default()
                };
                if SendMessageW(
                    s.tab,
                    TCM_INSERTITEMW,
                    Some(WPARAM(index)),
                    Some(LPARAM(&item as *const _ as isize)),
                )
                .0 == -1
                {
                    return Err("設定タブを作成できません".into());
                }
            }
            s.build_basic_page(settings, inputs, outputs)?;
            s.build_keys_page(settings)?;
            s.build_rvc_page(settings, schema)?;
            s.build_sounds_page(settings, outputs)?;
            for (page, id) in [RESET_BASIC, RESET_KEYS, RESET_RVC, RESET_SOUNDS]
                .into_iter()
                .enumerate()
            {
                s.control(
                    "BUTTON",
                    "初期値に戻す",
                    BS_PUSHBUTTON as u32,
                    id,
                    CLIENT_WIDTH - 180,
                    568,
                    145,
                    30,
                    Some(page),
                )?;
            }
            s.refresh_rvc_controls();
            s.message = s.control("STATIC", "", 0, 400, 20, 624, CLIENT_WIDTH - 45, 22, None)?;
            s.save = s.control(
                "BUTTON",
                "適用",
                BS_DEFPUSHBUTTON as u32,
                SAVE,
                CLIENT_WIDTH - 285,
                655,
                145,
                30,
                None,
            )?;
            s.cancel = s.control(
                "BUTTON",
                "閉じる",
                BS_PUSHBUTTON as u32,
                CANCEL,
                CLIENT_WIDTH - 130,
                655,
                105,
                30,
                None,
            )?;
            s.page(0);
            ACTIONS.lock().unwrap().clear();
            WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
            s.show();
            Ok(s)
        }
    }
    fn build_sounds_page(
        &mut self,
        settings: &Settings,
        outputs: &[AudioDeviceInfo],
    ) -> Result<(), String> {
        let labels = [
            "MICへ切替",
            "MIC-RVCへ切替",
            "SPK-RVCへ切替",
            "送信停止へ切替",
            "出力1切替",
            "出力2切替",
            "出力3切替",
        ];
        self.label("効果音の再生デバイス", 20, 80, 170, 28, 3)?;
        let devices = device_choices(
            outputs,
            &settings.sound_device_id,
            &settings.sound_device_name,
        );
        let names: Vec<String> = devices.iter().map(|(_, name)| name.clone()).collect();
        let index = devices
            .iter()
            .position(|(id, _)| *id == settings.sound_device_id)
            .unwrap_or(0);
        let handle = self.combo(
            &names,
            names.get(index).map_or("", String::as_str),
            false,
            190,
            80,
            545,
            510,
            3,
        )?;
        select(handle, index);
        self.sound_device = Some(DeviceControl {
            handle,
            devices,
            disconnected_choice: None,
        });
        let choices: Vec<String> = crate::sounds::LABELS.iter().map(|s| (*s).into()).collect();
        for (i, label) in labels.iter().enumerate() {
            let y = 132 + i as i32 * 50;
            self.label(label, 20, y, 170, 28, 3)?;
            let selected = usize::from(settings.switch_sounds[i]);
            self.sounds[i] = self.combo(
                &choices,
                choices.get(selected).map_or("なし", String::as_str),
                false,
                190,
                y,
                250,
                500 + i,
                3,
            )?;
        }
        Ok(())
    }

    fn build_basic_page(
        &mut self,
        settings: &Settings,
        inputs: &[AudioDeviceInfo],
        outputs: &[AudioDeviceInfo],
    ) -> Result<(), String> {
        self.label("オーバーレイ表示", 20, 80, 165, 28, 0)?;
        self.overlay = self.combo(
            &OverlayMode::ALL.map(|m| m.label().into()),
            settings.overlay_mode.label(),
            false,
            190,
            80,
            545,
            200,
            0,
        )?;
        self.label("ログイン時のモード", 20, 126, 165, 27, 0)?;
        let modes = vec![
            "前回のモード".into(),
            "MIC".into(),
            "MIC-RVC".into(),
            "SPK-RVC".into(),
            "送信停止".into(),
        ];
        self.startup = self.combo(&modes, "前回のモード", false, 190, 126, 545, 220, 0)?;
        self.startup_modes = vec![
            None,
            Some(Mode::Direct),
            Some(Mode::Rvc),
            Some(Mode::SpeakerRvc),
            Some(Mode::Muted),
        ];
        select(
            self.startup,
            match settings.startup_mode {
                None => 0,
                Some(Mode::Direct) => 1,
                Some(Mode::Rvc) => 2,
                Some(Mode::SpeakerRvc) => 3,
                Some(Mode::Muted) => 4,
            },
        );
        self.expand_devices = self.control(
            "BUTTON",
            "未選択の欄を表示",
            BS_PUSHBUTTON as u32,
            SHOW_UNSELECTED,
            748,
            126,
            177,
            28,
            Some(0),
        )?;
        self.rvc_enabled = self.checkbox(
            "RVCを使用する",
            settings.rvc_enabled,
            20,
            165,
            210,
            RVC_ENABLED,
            0,
        )?;
        self.tip(self.rvc_enabled, "ONにするとRVC設定タブを開けます。OFFにするとRVC関連の操作を無効にします。稼働状態は「適用」で切り替わります。")?;
        for (i, (label, devices, id, name, enabled)) in [
            (
                "入力マイク(原音/RVC)",
                inputs,
                &settings.microphone_id,
                &settings.microphone_name,
                None,
            ),
            (
                "入力スピーカー(RVC)",
                outputs,
                &settings.speaker_id,
                &settings.speaker_name,
                None,
            ),
            (
                "出力1（配信用）",
                outputs,
                &settings.cable_id,
                &settings.cable_name,
                Some((0, settings.cable_enabled)),
            ),
            (
                "出力2（音声入力用）",
                outputs,
                &settings.voice_input_id,
                &settings.voice_input_name,
                Some((1, settings.voice_input_enabled)),
            ),
            (
                "出力3（モニタ用）",
                outputs,
                &settings.monitor_id,
                &settings.monitor_name,
                Some((2, settings.monitoring)),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let y = 202 + i as i32 * 43;
            let start = self.pages[0].len();
            if let Some((output, _)) = enabled {
                self.label(&format!("出力{}", output + 1), 20, y, 43, 27, 0)?;
                self.output_labels[output] = self.control(
                    "EDIT",
                    &settings.output_labels[output],
                    WS_BORDER.0 | ES_AUTOHSCROLL as u32,
                    240 + output,
                    65,
                    y,
                    120,
                    27,
                    Some(0),
                )?;
                unsafe {
                    SendMessageW(
                        self.output_labels[output],
                        0x00c5, // EM_SETLIMITTEXT
                        Some(WPARAM(48)),
                        None,
                    );
                }
            } else {
                self.label(label, 20, y, 165, 27, 0)?;
            }
            let name = name.trim_end_matches("（未接続）");
            let mut choices = device_choices(devices, id, name);
            choices.insert(0, (String::new(), UNSELECTED.into()));
            let labels: Vec<_> = choices.iter().map(|(_, name)| name.clone()).collect();
            // 未接続項目も名前順に含める。注記は表示専用で、保存名へ戻さない。
            let disconnected = !id.is_empty()
                && !is_default_device(id)
                && !devices
                    .iter()
                    .any(|d| d.state == "Active" && d.endpoint_id == *id);
            if disconnected {
                if let Some((_, saved_name)) = choices.iter_mut().find(|(key, _)| key == id) {
                    *saved_name = name.into();
                }
            }
            let selected = choices.iter().position(|(key, _)| key == id).unwrap_or(0);
            let combo = self.combo(
                &labels,
                labels.get(selected).map_or("", String::as_str),
                false,
                190,
                y,
                545,
                210 + i,
                0,
            )?;
            select(combo, selected);
            self.devices.push(DeviceControl {
                handle: combo,
                devices: choices,
                disconnected_choice: disconnected.then_some(selected),
            });
            if let Some((output, on)) = enabled {
                self.outputs[output] = self.checkbox("有効", on, 748, y, 67, 230 + output, 0)?;
                self.tip(combo, "「未接続」のデバイスは選択先として保持されます。接続を確認して設定画面を開き直すと、出力を有効にできます。")?;
            }
            self.device_rows[i].extend_from_slice(&self.pages[0][start..]);
        }
        self.refresh_startup_modes();
        self.noise = self.checkbox(
            "短いノイズを抑制（RVC）",
            settings.noise_suppression(),
            20,
            444,
            350,
            221,
            0,
        )?;
        self.tip(self.noise, super::tooltip::NOISE_HELP)?;
        let link = self.control(
            "BUTTON",
            "公開GitHub",
            BS_OWNERDRAW as u32,
            260,
            20,
            492,
            560,
            28,
            Some(0),
        )?;
        unsafe {
            SendMessageW(
                link,
                WM_SETFONT,
                Some(WPARAM(self.link_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        Ok(())
    }

    fn build_keys_page(&mut self, settings: &Settings) -> Result<(), String> {
        let key_choices: Vec<String> = std::iter::once("なし".into())
            .chain((b'A'..=b'Z').map(|c| (c as char).to_string()))
            .chain((1..=24).map(|n| format!("F{n}")))
            .collect();
        let modifier_choices = ["不要", "左", "右", "左|右", "左&右"].map(str::to_owned);
        let combo_width = 100;
        let column_x = |column| 292 + column * (combo_width + 6);
        let modifier_columns = [
            (2, "Ctrl", column_x(0)),
            (1, "Shift", column_x(1)),
            (4, "Alt", column_x(2)),
            (3, "Windows", column_x(3)),
        ];
        for (_, label, x) in modifier_columns {
            self.label(label, x, 54, combo_width, 21, 1)?;
        }
        self.label("キー", column_x(4), 54, combo_width, 21, 1)?;
        self.label("モード", column_x(5), 54, combo_width, 21, 1)?;
        for (row, &i) in display_order().iter().enumerate() {
            // 表示順とタブ移動順を揃え、保存時には従来の添字を維持する。
            let y = 78
                + row as i32 * 29
                + if row >= 3 { 14 } else { 0 }
                + if row >= 8 { 14 } else { 0 };
            if matches!(row, 3 | 8) {
                self.control(
                    "STATIC",
                    "",
                    0x0010, // SS_ETCHEDHORZ
                    270 + row,
                    20,
                    y - 9,
                    CLIENT_WIDTH - 45,
                    2,
                    Some(1),
                )?;
            }
            self.shortcut_titles[i] =
                self.label(&shortcut_title(settings, i), 20, y, 215, 28, 1)?;
            let k = &settings.hotkeys[i];
            let mut controls = [HWND::default(); 7];
            // 列IDと保存順は維持し、修飾キーの表示順だけCtrl、Shift、Alt、Windowsにする。
            controls[0] =
                self.checkbox("有効", k.is_assigned(), 235, y, 55, HOTKEY_BASE + i * 10, 1)?;
            for ((j, _, x), (on, side)) in modifier_columns.into_iter().zip([
                (k.ctrl, k.ctrl_side),
                (k.shift, k.shift_side),
                (k.alt, k.alt_side),
                (k.win, k.win_side),
            ]) {
                let selected = modifier_index(on, side);
                controls[j] = self.combo(
                    &modifier_choices,
                    &modifier_choices[selected],
                    false,
                    x,
                    y,
                    combo_width,
                    HOTKEY_BASE + i * 10 + j,
                    1,
                )?;
                self.tip(
                    controls[j],
                    "左|右はどちらか一方以上、左&右は左右同時です。不要は割当に含めません。",
                )?;
            }
            controls[5] = self.combo(
                &key_choices,
                primary_key_label(k),
                false,
                column_x(4),
                y,
                combo_width,
                HOTKEY_BASE + 5 + i * 10,
                1,
            )?;
            controls[6] = self.combo(
                &KeyHandling::ALL.map(|mode| mode.label().into()),
                k.handling.label(),
                false,
                column_x(5),
                y,
                combo_width,
                HOTKEY_BASE + 6 + i * 10,
                1,
            )?;
            self.tip(controls[6], "共存は他のアプリにもキーを渡します。上書きは主キーを抑止します。修飾キーだけの押している間ONは共存です。")?;
            self.keys[i] = controls;
        }
        Ok(())
    }

    fn build_rvc_page(
        &mut self,
        settings: &Settings,
        schema: Vec<ParameterDescriptor>,
    ) -> Result<(), String> {
        self.label(
            "＊の項目を変更するとRVCを再起動します。その他は起動中にも反映します。",
            20,
            65,
            800,
            28,
            2,
        )?;
        let mut file_row = 0;
        let mut other_row = 0;
        // 同じ設定値を基本設定とRVC設定の両方で編集させない。
        for (i, p) in schema
            .into_iter()
            .filter(|p| p.id != NOISE_PARAMETER)
            .enumerate()
        {
            let value = settings.rvc_parameters.get(&p.id).unwrap_or(&p.default);
            let label = parameter_label(&p);
            let handle = if p.kind == ParameterKind::File {
                let y = 100 + file_row * 42;
                file_row += 1;
                let title = self.label(&label, 20, y, 180, 26, 2)?;
                self.tip(title, &parameter_help(&p))?;
                self.control(
                    "BUTTON",
                    "フォルダーを開く",
                    BS_PUSHBUTTON as u32,
                    450 + i,
                    740,
                    y,
                    190,
                    28,
                    Some(2),
                )?;
                self.combo(
                    &p.options,
                    value.as_str().unwrap_or(""),
                    true,
                    210,
                    y,
                    520,
                    300 + i,
                    2,
                )?
            } else {
                let x = 20 + (other_row % 2) * 405;
                let y = 190 + (other_row / 2) * 48;
                other_row += 1;
                if p.kind == ParameterKind::Boolean {
                    self.checkbox(
                        &label,
                        value.as_bool().unwrap_or(false),
                        x,
                        y,
                        390,
                        300 + i,
                        2,
                    )?
                } else {
                    let title = self.label(&label, x, y, 244, 24, 2)?;
                    self.tip(title, &parameter_help(&p))?;
                    let handle = if p.kind == ParameterKind::Enum {
                        self.combo(
                            &p.options,
                            value.as_str().unwrap_or(""),
                            false,
                            x + 246,
                            y,
                            132,
                            300 + i,
                            2,
                        )?
                    } else {
                        self.control(
                            "EDIT",
                            &parameter_text(value),
                            WS_BORDER.0 | ES_AUTOHSCROLL as u32,
                            300 + i,
                            x + 246,
                            y,
                            132,
                            26,
                            Some(2),
                        )?
                    };
                    handle
                }
            };
            self.tip(handle, &parameter_help(&p))?;
            self.parameters.push(ParameterControl {
                schema: p,
                handle,
                original_input: (text(handle), checked(handle)),
            });
        }
        self.label(
            "モデルとIndexは任意のフルパスを直接入力・貼り付けできます。相対パスはプラグイン基準です。",
            20,
            568,
            745,
            25,
            2,
        )?;
        Ok(())
    }

    pub(super) fn page(&self, index: usize) {
        if index >= self.pages.len() {
            return;
        }
        let index = if index == RVC_PAGE && !checked(self.rvc_enabled) {
            let current = self.active_page.get();
            if current == RVC_PAGE {
                0
            } else {
                current
            }
        } else {
            index
        };
        self.active_page.set(index);
        if let Some(tips) = &self.tips {
            tips.pop();
        }
        unsafe {
            SendMessageW(self.tab, TCM_SETCURSEL, Some(WPARAM(index)), None);
        }

        for (i, controls) in self.pages.iter().enumerate() {
            for h in controls {
                unsafe {
                    let _ = ShowWindow(*h, if i == index { SW_SHOW } else { SW_HIDE });
                }
            }
        }
        if index == 0 {
            self.layout_device_rows();
        }
    }
    fn selected_devices(&self) -> DeviceSelection {
        let selected = |index: usize| selection(self.devices[index].handle) != 0;
        DeviceSelection {
            microphone: selected(0),
            speaker: selected(1),
            outputs: std::array::from_fn(|i| selected(i + 2)),
        }
    }
    fn layout_device_rows(&self) {
        let current = self.selected_devices();
        let saved = self.base.device_selection();
        let assigned = [
            current.microphone,
            current.speaker,
            current.outputs[0],
            current.outputs[1],
            current.outputs[2],
        ];
        let previously_assigned = [
            saved.microphone,
            saved.speaker,
            saved.outputs[0],
            saved.outputs[1],
            saved.outputs[2],
        ];
        let mut y = 202;
        for (i, controls) in self.device_rows.iter().enumerate() {
            let visible = assigned[i] || previously_assigned[i] || self.show_unselected;
            for control in controls {
                unsafe {
                    let mut rect = RECT::default();
                    let _ = GetWindowRect(*control, &mut rect);
                    let mut pos = windows::Win32::Foundation::POINT {
                        x: rect.left,
                        y: rect.top,
                    };
                    let _ = ScreenToClient(self.hwnd, &mut pos);
                    let _ = SetWindowPos(
                        *control,
                        None,
                        pos.x,
                        y,
                        0,
                        0,
                        SWP_NOSIZE
                            | SWP_NOZORDER
                            | SWP_NOACTIVATE
                            | if visible {
                                SWP_SHOWWINDOW
                            } else {
                                SWP_HIDEWINDOW
                            },
                    );
                }
            }
            if visible {
                y += 43;
            }
        }
        set_text(
            self.expand_devices,
            if self.show_unselected {
                "未選択の欄を隠す"
            } else {
                "未選択の欄を表示"
            },
        );
        unsafe {
            let _ = ShowWindow(
                self.expand_devices,
                if assigned.iter().all(|selected| *selected) {
                    SW_HIDE
                } else {
                    SW_SHOW
                },
            );
        }
    }
    fn refresh_startup_modes(&mut self) {
        let selected = self
            .startup_modes
            .get(selection(self.startup))
            .copied()
            .flatten();
        let devices = self.selected_devices();
        let rvc_enabled = checked(self.rvc_enabled);
        self.startup_modes = [
            None,
            Some(Mode::Direct),
            Some(Mode::Rvc),
            Some(Mode::SpeakerRvc),
            Some(Mode::Muted),
        ]
        .into_iter()
        .filter(|mode| {
            *mode == selected
                || mode.is_none_or(|mode| {
                    (rvc_enabled || !mode.is_rvc()) && devices.has_source(mode)
                })
        })
        .collect();
        unsafe {
            SendMessageW(self.startup, CB_RESETCONTENT, None, None);
            for mode in &self.startup_modes {
                let label = match mode {
                    Some(mode) if !rvc_enabled && mode.is_rvc() => "送信停止（RVC無効）",
                    Some(mode) => mode.label(),
                    None => "前回のモード",
                };
                SendMessageW(
                    self.startup,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(wide(label).as_ptr() as isize)),
                );
            }
        }
        select(
            self.startup,
            self.startup_modes
                .iter()
                .position(|mode| *mode == selected)
                .unwrap_or(0),
        );
    }
    pub fn show(&self) {
        unsafe {
            // 表示済みの窓を復元し直すと、Win32のアクティブ化処理が再び走る。
            // 明示的な再表示要求でも、編集中のコントロールはそのまま保つ。
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            } else if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
                // STARTUPINFOのSW_HIDEが最初のShowWindowを上書きする場合も、明示表示を完了する。
                if !IsWindowVisible(self.hwnd).as_bool() {
                    let _ = ShowWindow(self.hwnd, SW_SHOW);
                }
            }
            if GetForegroundWindow() != self.hwnd {
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
    }
    pub fn busy(&mut self, on: bool) {
        self.busy = on;
        unsafe {
            let _ = EnableWindow(self.tab, !on);
            let _ = EnableWindow(self.save, on == false);
            let _ = EnableWindow(self.cancel, on == false);
        }
        for controls in &self.pages {
            for h in controls {
                unsafe {
                    let _ = EnableWindow(*h, !on);
                }
            }
        }
        self.refresh_hotkey_controls();
        self.refresh_rvc_controls();
        self.message(if on {
            "設定を適用しています…"
        } else {
            ""
        });
    }
    fn reset_basic(&mut self) {
        let mut defaults = self.base.clone();
        defaults.reset_basic();
        select_label(self.overlay, defaults.overlay_mode.label());
        select(self.startup, 0);
        for (device, id) in self.devices.iter().zip([
            &defaults.microphone_id,
            &defaults.speaker_id,
            &defaults.cable_id,
            &defaults.voice_input_id,
            &defaults.monitor_id,
        ]) {
            if let Some(index) = device
                .devices
                .iter()
                .position(|(candidate, _)| candidate == id)
            {
                select(device.handle, index);
            }
        }
        for (handle, label) in self.output_labels.into_iter().zip(&defaults.output_labels) {
            set_text(handle, label);
        }
        for handle in self.outputs {
            set_checked(handle, false);
        }
        set_checked(self.rvc_enabled, defaults.rvc_enabled);
        set_checked(self.noise, defaults.noise_suppression());
        self.refresh_startup_modes();
        self.refresh_rvc_controls();
        self.layout_device_rows();
        self.message("基本設定を初期値に戻しました。「適用」で反映します。");
    }
    fn reset_rvc(&mut self) {
        let defaults = initial_rvc_parameters();
        for parameter in &self.parameters {
            let value = defaults
                .get(&parameter.schema.id)
                .unwrap_or(&parameter.schema.default);
            match parameter.schema.kind {
                ParameterKind::Boolean => {
                    set_checked(parameter.handle, value.as_bool().unwrap_or(false))
                }
                ParameterKind::Enum => select_label(parameter.handle, &parameter_text(value)),
                _ => set_text(parameter.handle, &parameter_text(value)),
            }
        }
        self.rvc_reset = true;
        self.message("RVC設定を初期値に戻しました。「適用」で反映します。");
    }
    fn reset_keys(&self) {
        for (row, key) in self.keys.iter().zip(default_hotkeys()) {
            set_checked(row[0], key.is_assigned());
            select(row[1], modifier_index(key.shift, key.shift_side));
            select(row[2], modifier_index(key.ctrl, key.ctrl_side));
            select(row[3], modifier_index(key.win, key.win_side));
            select(row[4], modifier_index(key.alt, key.alt_side));
            select_label(row[5], primary_key_label(&key));
            select_label(row[6], key.handling.label());
        }
        self.refresh_hotkey_controls();
        self.message("キー設定を初期値に戻しました。「適用」で反映します。");
    }
    fn reset_sounds(&self) {
        let mut defaults = self.base.clone();
        defaults.reset_sounds();
        for (handle, sound) in self.sounds.into_iter().zip(defaults.switch_sounds) {
            select(handle, usize::from(sound));
        }
        if let Some(device) = &self.sound_device {
            if let Some(index) = device
                .devices
                .iter()
                .position(|(id, _)| id == &defaults.sound_device_id)
            {
                select(device.handle, index);
            }
        }
        self.message("効果音設定を初期値に戻しました。「適用」で反映します。");
    }
    fn refresh_hotkey_controls(&self) {
        let devices = self.selected_devices();
        for (index, row) in self.keys.iter().enumerate() {
            let command = crate::actions::HOTKEY_COMMANDS[index];
            let available = !self.busy
                && devices.permits(command)
                && (checked(self.rvc_enabled) || !crate::actions::requires_rvc(command));
            let enabled = available && checked(row[0]);
            let modifier_only = crate::actions::ptt_event(command).is_some() && selection(row[5]) == 0;
            if enabled && modifier_only {
                select(row[6], 0);
            }
            unsafe {
                let _ = EnableWindow(row[0], available);
                let _ = EnableWindow(self.shortcut_titles[index], enabled);
                for control in &row[1..] {
                    let _ = EnableWindow(*control, enabled);
                }
                let _ = EnableWindow(row[6], enabled && !modifier_only);
            }
        }
    }
    fn refresh_rvc_controls(&self) {
        let available = checked(self.rvc_enabled);
        let enabled = !self.busy && available;
        if !available && self.active_page.get() == RVC_PAGE {
            self.page(0);
        }
        unsafe {
            let mut rect = RECT::default();
            if SendMessageW(
                self.tab,
                TCM_GETITEMRECT,
                Some(WPARAM(RVC_PAGE)),
                Some(LPARAM(&mut rect as *mut _ as isize)),
            )
            .0 != 0
            {
                let _ = InvalidateRect(Some(self.tab), Some(&rect), false);
            }
            for control in &self.pages[RVC_PAGE] {
                let _ = EnableWindow(*control, enabled);
            }
            let _ = EnableWindow(self.noise, enabled);
            let _ = EnableWindow(self.devices[1].handle, enabled);
            for (i, selected) in self.selected_devices().outputs.into_iter().enumerate() {
                if !selected {
                    set_checked(self.outputs[i], false);
                }
                let device = &self.devices[i + 2];
                let connected = device.disconnected_choice != Some(selection(device.handle));
                let can_toggle = connected || checked(self.outputs[i]);
                let _ = EnableWindow(self.outputs[i], !self.busy && selected && can_toggle);
            }
        }
        self.refresh_hotkey_controls();
    }
    pub fn shortcut_pending(&self, pending: bool) {
        // 入力中の欄やフォーカスには触れず、競合する適用だけを待たせる。
        unsafe {
            let _ = EnableWindow(self.save, !pending && !self.busy);
        }
    }
    pub fn sync_shortcut(&mut self, action: usize, settings: &Settings) {
        self.base = settings.clone();
        let selected = self.selected_devices().outputs;
        match action {
            crate::actions::OVERLAY => select(
                self.overlay,
                OverlayMode::ALL
                    .iter()
                    .position(|m| *m == settings.overlay_mode)
                    .unwrap(),
            ),
            crate::actions::OUTPUT1 => {
                set_checked(self.outputs[0], selected[0] && settings.cable_enabled)
            }
            crate::actions::OUTPUT2 => {
                set_checked(self.outputs[1], selected[1] && settings.voice_input_enabled)
            }
            crate::actions::MONITOR => {
                set_checked(self.outputs[2], selected[2] && settings.monitoring)
            }
            crate::actions::NOISE => set_checked(self.noise, settings.noise_suppression()),
            _ => (),
        }
        self.shortcut_pending(false);
    }
    pub fn applied(&mut self, settings: &Settings) {
        self.base = settings.clone();
        self.rvc_reset = false;
        for p in &mut self.parameters {
            p.original_input = (text(p.handle), checked(p.handle));
        }
        self.show_unselected = false;
        if unsafe { SendMessageW(self.tab, TCM_GETCURSEL, None, None).0 } == 0 {
            self.layout_device_rows();
        }
        // 「有効」だけONでキーが「なし」の場合も、保存結果へ表示を合わせる。
        for (row, key) in self.keys.iter().zip(&settings.hotkeys) {
            set_checked(row[0], key.is_assigned());
        }
        self.busy(false);
        self.message("適用しました");
    }
    pub fn message(&self, message: &str) {
        set_text(self.message, message);
    }
    pub(super) fn read(&self) -> Result<Settings, String> {
        let mut settings = self.base.clone();
        settings.device_selection_revision = 1;
        settings.rvc_enabled = checked(self.rvc_enabled);
        let device = self
            .sound_device
            .as_ref()
            .ok_or("効果音の再生デバイスがありません")?;
        (settings.sound_device_id, settings.sound_device_name) = device
            .devices
            .get(selection(device.handle))
            .cloned()
            .ok_or("効果音の再生デバイスを選択してください")?;
        for (i, handle) in self.sounds.iter().enumerate() {
            settings.switch_sounds[i] =
                u8::try_from(selection(*handle)).map_err(|_| "効果音を選択してください")?;
        }
        for (i, row) in self.keys.iter().enumerate() {
            let key = text(row[5]);
            let (shift, shift_side) = read_modifier(row[1])?;
            let (ctrl, ctrl_side) = read_modifier(row[2])?;
            let (win, win_side) = read_modifier(row[3])?;
            let (alt, alt_side) = read_modifier(row[4])?;
            let ptt = crate::actions::ptt_event(crate::actions::HOTKEY_COMMANDS[i]).is_some();
            let modifier_only = ptt && key == "なし" && (shift || ctrl || win || alt);
            if checked(row[0]) && ptt && key == "なし" && !modifier_only {
                return Err("押している間ONにはキーまたは修飾キーを指定してください".into());
            }
            settings.hotkeys[i] = Hotkey {
                handling: if modifier_only {
                    KeyHandling::Coexist
                } else {
                    *KeyHandling::ALL
                        .get(selection(row[6]))
                        .ok_or("共存または上書きを選択してください")?
                },
                enabled: checked(row[0]) && (key != "なし" || modifier_only),
                shift,
                ctrl,
                win,
                alt,
                key: if key == "なし" { "None".into() } else { key },
                shift_side,
                ctrl_side,
                win_side,
                alt_side,
                modifier_only,
            };
        }
        settings.overlay_mode = *OverlayMode::ALL
            .get(selection(self.overlay))
            .ok_or("オーバーレイの表示方法を選択してください")?;
        settings.output_labels = self
            .output_labels
            .map(|handle| text(handle).trim().to_owned());
        settings.cable_enabled = checked(self.outputs[0]);
        settings.voice_input_enabled = checked(self.outputs[1]);
        settings.monitoring = checked(self.outputs[2]);
        if checked(self.noise) != self.base.noise_suppression() {
            settings.set_noise_suppression(checked(self.noise));
        }
        let mut chosen = Vec::new();
        for d in &self.devices {
            chosen.push(
                d.devices
                    .get(selection(d.handle))
                    .cloned()
                    .ok_or("デバイスを選択してください")?,
            );
        }
        (settings.microphone_id, settings.microphone_name) = chosen[0].clone();
        (settings.speaker_id, settings.speaker_name) = chosen[1].clone();
        (settings.cable_id, settings.cable_name) = chosen[2].clone();
        (settings.voice_input_id, settings.voice_input_name) = chosen[3].clone();
        (settings.monitor_id, settings.monitor_name) = chosen[4].clone();
        settings.startup_mode = self
            .startup_modes
            .get(selection(self.startup))
            .copied()
            .flatten();
        let selected = settings.device_selection().outputs;
        settings.cable_enabled &= selected[0];
        settings.voice_input_enabled &= selected[1];
        settings.monitoring &= selected[2];
        for p in &self.parameters {
            let input = (text(p.handle), checked(p.handle));
            // 無効のまま開いた旧設定は検証し直さず、編集とリセットはOFFでも保存する。
            if !settings.rvc_enabled && !self.rvc_reset && input == p.original_input {
                continue;
            }
            let value = parse_parameter(&p.schema, &input.0, input.1)?;
            let previous = self
                .base
                .rvc_parameters
                .get(&p.schema.id)
                .unwrap_or(&p.schema.default);
            if self.rvc_reset || parameter_value_changed(&value, previous) {
                settings.rvc_parameters[&p.schema.id] = value;
            }
        }
        settings.validate()?;
        Ok(settings)
    }
    pub fn poll(&mut self) -> Option<Action> {
        let actions = std::mem::take(&mut *ACTIONS.lock().unwrap());
        for action in actions {
            if self.busy {
                continue;
            }
            match action {
                SAVE => match self.read() {
                    Ok(s) => return Some(Action::Save(Box::new(s))),
                    Err(e) => self.message(&e),
                },
                CANCEL => return Some(Action::Close),
                RESET_BASIC => self.reset_basic(),
                RESET_KEYS => self.reset_keys(),
                RESET_RVC if checked(self.rvc_enabled) => self.reset_rvc(),
                RESET_SOUNDS => self.reset_sounds(),
                SHOW_UNSELECTED => {
                    self.show_unselected = !self.show_unselected;
                    self.layout_device_rows();
                }
                210..=214 => {
                    if action <= 211 {
                        self.refresh_startup_modes();
                    } else {
                        let device = &self.devices[action - 210];
                        if device.disconnected_choice == Some(selection(device.handle)) {
                            set_checked(self.outputs[action - 212], false);
                        }
                    }
                    self.refresh_rvc_controls();
                    self.layout_device_rows();
                }
                220 => self.refresh_startup_modes(),
                230..=232 => self.refresh_rvc_controls(),
                10 => self.page(0),
                11 => self.page(1),
                12 => self.page(2),
                13 => self.page(3),
                id @ 500..=506 => {
                    let device = self.sound_device.as_ref()?;
                    let endpoint = device.devices.get(selection(device.handle))?.0.clone();
                    return Some(Action::Preview(
                        selection(self.sounds[id - 500]) as u8,
                        endpoint,
                    ));
                }
                RVC_ENABLED => {
                    self.refresh_startup_modes();
                    self.refresh_rvc_controls();
                }
                260 => {
                    if let Err(error) = super::help::open_repository() {
                        self.message(&error);
                    }
                }
                id if (450..450 + self.parameters.len()).contains(&id) => {
                    if let Some(parameter) = self.parameters.get(id - 450) {
                        if parameter.schema.kind == ParameterKind::File {
                            let result = super::help::model_folder(
                                &self.base.plugin_directory.join("rvc-cuda"),
                                &text(parameter.handle),
                                parameter.schema.default.as_str().unwrap_or(""),
                            )
                            .and_then(|folder| super::help::open_folder(&folder));
                            if let Err(error) = result {
                                self.message(&error);
                            }
                        }
                    }
                }
                240..=242 => {
                    let mut edited = self.base.clone();
                    edited.output_labels = self.output_labels.map(text);
                    for i in 0..HOTKEY_COUNT {
                        set_text(self.shortcut_titles[i], &shortcut_title(&edited, i));
                    }
                }
                id if id >= HOTKEY_BASE
                    && id < HOTKEY_BASE + HOTKEY_COUNT * 10
                    && (id - HOTKEY_BASE) % 10 == 0 =>
                {
                    self.refresh_hotkey_controls()
                }
                id if id >= HOTKEY_BASE + 5
                    && id < HOTKEY_BASE + HOTKEY_COUNT * 10
                    && (id - HOTKEY_BASE - 5) % 10 == 0 =>
                {
                    if let Some(row) = self.keys.get((id - HOTKEY_BASE - 5) / 10) {
                        let index = (id - HOTKEY_BASE - 5) / 10;
                        if selection(row[5]) == 0
                            && crate::actions::ptt_event(crate::actions::HOTKEY_COMMANDS[index])
                                .is_none()
                        {
                            set_checked(row[0], false);
                        }
                    }
                    self.refresh_hotkey_controls();
                }
                _ => {}
            }
        }
        None
    }
}

fn primary_key_label(key: &Hotkey) -> &str {
    if key.key == "None" {
        "なし"
    } else {
        &key.key
    }
}

fn modifier_index(enabled: bool, side: ModifierSide) -> usize {
    if !enabled {
        0
    } else {
        match side {
            ModifierSide::Left => 1,
            ModifierSide::Right => 2,
            ModifierSide::Either => 3,
            ModifierSide::Both => 4,
        }
    }
}
fn read_modifier(handle: HWND) -> Result<(bool, ModifierSide), String> {
    match selection(handle) {
        0 => Ok((false, ModifierSide::Either)),
        1 => Ok((true, ModifierSide::Left)),
        2 => Ok((true, ModifierSide::Right)),
        3 => Ok((true, ModifierSide::Either)),
        4 => Ok((true, ModifierSide::Both)),
        _ => Err("修飾キーの左右を選択してください".into()),
    }
}

fn shortcut_title(settings: &Settings, index: usize) -> String {
    if matches!(
        crate::actions::HOTKEY_COMMANDS[index],
        crate::actions::PTT1 | crate::actions::PTT2 | crate::actions::PTT3
    ) {
        return "    押している間ON".into();
    }

    let output = match crate::actions::HOTKEY_COMMANDS[index] {
        crate::actions::OUTPUT1 => Some(0),
        crate::actions::OUTPUT2 => Some(1),
        crate::actions::MONITOR => Some(2),
        _ => None,
    };
    output.map_or_else(
        || HOTKEY_LABELS[index].into(),
        |i| format!("{}切替", settings.output_label(i)),
    )
}
