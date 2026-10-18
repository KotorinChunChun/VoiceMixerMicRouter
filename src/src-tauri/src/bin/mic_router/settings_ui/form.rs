//! 基本設定とRVC設定の構築・読み取り・操作。
use super::super::{
    config::{
        device_choices, is_default_device, Hotkey, KeyHandling, Mode, OverlayMode, Settings,
        HOTKEY_COUNT, HOTKEY_LABELS, NOISE_PARAMETER,
    },
    ui,
};
use super::{
    controls::{checked, select, selection, set_checked, set_text, text, wide},
    display_order,
    parameters::{
        parameter_help, parameter_label, parameter_text, parameter_value_changed, parse_parameter,
    },
    Action, DeviceControl, ParameterControl, SettingsWindow, ACTIONS, CANCEL, CLASS, CLIENT_HEIGHT,
    CLIENT_WIDTH, HOTKEY_BASE, SAVE, TAB, WINDOW,
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
        UI::{Controls::*, Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
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
                keys: vec![[HWND::default(); 7]; HOTKEY_COUNT],
                devices: Vec::new(),
                parameters: Vec::new(),
                overlay: HWND::default(),
                outputs: [HWND::default(); 3],
                output_labels: [HWND::default(); 3],
                shortcut_titles: vec![HWND::default(); HOTKEY_COUNT],
                noise: HWND::default(),
                rvc_enabled: HWND::default(),
                startup: HWND::default(),
                save: HWND::default(),
                cancel: HWND::default(),
                message: HWND::default(),
                busy: false,
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
                TCS_FIXEDWIDTH | WS_CLIPSIBLINGS.0,
                TAB,
                10,
                16,
                CLIENT_WIDTH - 20,
                600,
                None,
            )?;
            SendMessageW(s.tab, TCM_SETITEMSIZE, None, Some(LPARAM((32 << 16) | 145)));
            for (index, title) in ["基本設定", "キー設定", "RVC設定"].iter().enumerate() {
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
        for (i, (label, devices, id, name, enabled)) in [
            (
                "入力マイク",
                inputs,
                &settings.microphone_id,
                &settings.microphone_name,
                None,
            ),
            (
                "入力スピーカー",
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
            let labels: Vec<_> = choices.iter().map(|(_, name)| name.clone()).collect();
            // 未接続項目も名前順に含める。注記は表示専用で、保存名へ戻さない。
            if !is_default_device(id)
                && !devices
                    .iter()
                    .any(|d| d.state == "Active" && d.endpoint_id == *id)
            {
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
            });
            if let Some((output, on)) = enabled {
                self.outputs[output] = self.checkbox("有効", on, 748, y, 67, 230 + output, 0)?;
            }
        }
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
        for (row, &i) in display_order().iter().enumerate() {
            // 表示順とタブ移動順を揃え、保存時には従来の添字を維持する。
            let y = 70
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
            // 列IDは旧設定・自動操作との互換のため保存し、作成順だけを表示順にする。
            for (j, label, on, x, width) in [
                (0, "有効", k.is_assigned(), 240, 60),
                (2, "Ctrl", k.ctrl, 310, 65),
                (1, "Shift", k.shift, 385, 72),
                (4, "Alt", k.alt, 468, 62),
                (3, "Windows", k.win, 540, 104),
            ]
            .into_iter()
            {
                controls[j] = self.checkbox(label, on, x, y, width, HOTKEY_BASE + i * 10 + j, 1)?;
            }
            let selected = if k.key == "None" { "なし" } else { &k.key };
            controls[5] = self.combo(
                &key_choices,
                selected,
                false,
                655,
                y,
                125,
                HOTKEY_BASE + 5 + i * 10,
                1,
            )?;
            controls[6] = self.combo(
                &KeyHandling::ALL.map(|mode| mode.label().into()),
                k.handling.label(),
                false,
                795,
                y,
                120,
                HOTKEY_BASE + 6 + i * 10,
                1,
            )?;
            self.keys[i] = controls;
        }
        self.label(
            "共存：このツールで操作し、同じキー入力をOSや他のアプリにも渡します。",
            20,
            520,
            905,
            28,
            1,
        )?;
        self.label(
            "上書き：このツールで操作し、一致した主キーをOSや他アプリへ渡さないようにします。",
            20,
            550,
            905,
            28,
            1,
        )?;
        self.label(
            "修飾キー単独やOSの保護された操作など、抑止できない入力があります。",
            20,
            578,
            905,
            28,
            1,
        )?;
        Ok(())
    }

    fn build_rvc_page(
        &mut self,
        settings: &Settings,
        schema: Vec<ParameterDescriptor>,
    ) -> Result<(), String> {
        self.rvc_enabled =
            self.checkbox("RVCを使用する", settings.rvc_enabled, 20, 65, 210, 222, 2)?;
        self.tip(self.rvc_enabled,
            "OFFにするとRVCを停止し、MICと送信停止だけを使用します。設定値は保持します。ONに戻した後、RVCモードを選ぶとモデルの準備を開始します。適用するまで稼働状態は変わりません。")?;
        self.label(
            "＊の項目を変更するとRVCを再起動します。その他は起動中にも反映します。",
            20,
            100,
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
                let y = 135 + file_row * 42;
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
                let y = 225 + (other_row / 2) * 48;
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
            self.parameters.push(ParameterControl { schema: p, handle });
        }
        self.label(
            "モデルとIndexは任意のフルパスを直接入力・貼り付けできます。相対パスはプラグイン基準です。",
            20,
            568,
            800,
            25,
            2,
        )?;
        Ok(())
    }

    pub(super) fn page(&self, index: usize) {
        if index >= self.pages.len() {
            return;
        }
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
    }
    pub fn show(&self) {
        unsafe {
            // 表示済みの窓を復元し直すと、Win32のアクティブ化処理が再び走る。
            // 明示的な再表示要求でも、編集中のコントロールはそのまま保つ。
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            } else if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
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
    fn refresh_hotkey_controls(&self) {
        for (index, row) in self.keys.iter().enumerate() {
            let available = !self.busy
                && (checked(self.rvc_enabled)
                    || !matches!(
                        crate::actions::HOTKEY_COMMANDS[index],
                        crate::actions::RVC | crate::actions::SPK_RVC | crate::actions::NOISE
                    ));
            let enabled = available && checked(row[0]);
            unsafe {
                let _ = EnableWindow(row[0], available);
                let _ = EnableWindow(self.shortcut_titles[index], enabled);
                for control in &row[1..] {
                    let _ = EnableWindow(*control, enabled);
                }
            }
        }
    }
    fn refresh_rvc_controls(&self) {
        let enabled = !self.busy && checked(self.rvc_enabled);
        unsafe {
            for control in &self.pages[2] {
                let _ = EnableWindow(
                    *control,
                    if *control == self.rvc_enabled {
                        !self.busy
                    } else {
                        enabled
                    },
                );
            }
            let _ = EnableWindow(self.noise, enabled);
            let _ = EnableWindow(self.devices[1].handle, enabled);
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
        match action {
            crate::actions::OVERLAY => select(
                self.overlay,
                OverlayMode::ALL
                    .iter()
                    .position(|m| *m == settings.overlay_mode)
                    .unwrap(),
            ),
            crate::actions::OUTPUT1 => set_checked(self.outputs[0], settings.cable_enabled),
            crate::actions::OUTPUT2 => set_checked(self.outputs[1], settings.voice_input_enabled),
            crate::actions::MONITOR => set_checked(self.outputs[2], settings.monitoring),
            crate::actions::NOISE => set_checked(self.noise, settings.noise_suppression()),
            _ => (),
        }
        self.shortcut_pending(false);
    }
    pub fn applied(&mut self, settings: &Settings) {
        self.base = settings.clone();
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
        settings.rvc_enabled = checked(self.rvc_enabled);
        for (i, row) in self.keys.iter().enumerate() {
            let key = text(row[5]);
            settings.hotkeys[i] = Hotkey {
                handling: *KeyHandling::ALL
                    .get(selection(row[6]))
                    .ok_or("共存または上書きを選択してください")?,
                enabled: checked(row[0]) && key != "なし",
                shift: checked(row[1]),
                ctrl: checked(row[2]),
                win: checked(row[3]),
                alt: checked(row[4]),
                key: if key == "なし" { "None".into() } else { key },
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
        settings.startup_mode = match selection(self.startup) {
            1 => Some(Mode::Direct),
            2 => Some(Mode::Rvc),
            3 => Some(Mode::SpeakerRvc),
            4 => Some(Mode::Muted),
            _ => None,
        };
        for p in self.parameters.iter().filter(|_| settings.rvc_enabled) {
            let value = parse_parameter(&p.schema, &text(p.handle), checked(p.handle))?;
            let previous = self
                .base
                .rvc_parameters
                .get(&p.schema.id)
                .unwrap_or(&p.schema.default);
            if parameter_value_changed(&value, previous) {
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
                10 => self.page(0),
                11 => self.page(1),
                12 => self.page(2),
                222 => self.refresh_rvc_controls(),
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
                        if selection(row[5]) == 0 {
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
