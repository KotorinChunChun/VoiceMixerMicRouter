//! WebView不要のトレイ、グローバルキー、フォーカスを奪わないモード表示。
use super::config::{Hotkey, Mode, OverlayMode, APP_NAME, APP_VERSION, HOTKEY_COUNT};
use super::hotkeys::RemoteHotkeys;
use super::telemetry::Snapshot;
use crate::actions::{shortcut_badge, shortcut_index, HOTKEY_COMMANDS};
pub use crate::actions::{
    DIRECT, EXIT, MONITOR, MUTED, NOISE, OUTPUT1, OUTPUT2, OVERLAY, RELEASE_RVC, RETRY, RVC,
    SETTINGS, SHOW, SPK_RVC, TOGGLE,
};
#[path = "ui/drawing.rs"]
mod drawing;
#[path = "ui/icon.rs"]
mod icon;
#[path = "ui/overlay.rs"]
mod overlay;
use icon::icon;
pub use icon::mode_icon;
use overlay::{Buttons, Overlay};
use std::sync::{Mutex, OnceLock};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};

const CLASS: &str = "VoiceMixerMicRouterV1";
const CALLBACK: u32 = WM_APP + 71;
const CONTROL: u32 = WM_APP + 72;
const SHOW_MENU: usize = 100;
const RESTORE: usize = 101;
static ACTIONS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
fn output_status(held: bool, enabled: bool) -> &'static str {
    if held {
        "P2T"
    } else if enabled {
        "ON"
    } else {
        "OFF"
    }
}
fn queue_action(action: usize) {
    ACTIONS.lock().unwrap().push(action);
}
static TASKBAR: OnceLock<u32> = OnceLock::new();
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn taskbar() -> u32 {
    *TASKBAR
        .get_or_init(|| unsafe { RegisterWindowMessageW(PCWSTR(wide("TaskbarCreated").as_ptr())) })
}

pub fn send_existing(action: usize) -> Result<bool, String> {
    unsafe {
        match FindWindowW(PCWSTR(wide(CLASS).as_ptr()), PCWSTR::null()) {
            Ok(hwnd) => {
                PostMessageW(Some(hwnd), CONTROL, WPARAM(action), LPARAM(0))
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}
pub fn error(message: &str) {
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide(message).as_ptr()),
            PCWSTR(wide(APP_NAME).as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let action = match msg {
        CONTROL | WM_COMMAND => Some(w.0 & 0xffff),
        WM_CLOSE => Some(EXIT),
        CALLBACK if matches!(l.0 as u32, WM_RBUTTONUP | WM_CONTEXTMENU) => Some(SHOW_MENU),
        CALLBACK if l.0 as u32 == WM_LBUTTONDBLCLK => Some(SETTINGS),
        _ if msg == taskbar() && msg != 0 => Some(RESTORE),
        _ => None,
    };
    if let Some(action) = action {
        queue_action(action);
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}
pub struct Ui {
    pub rvc_enabled: bool,
    hwnd: HWND,
    overlay: Overlay,
    data: NOTIFYICONDATAW,
    icons: Vec<HICON>,
    pub mode: Mode,
    pub hotkey: String,
    overlay_mode: OverlayMode,
    pub startup: Option<Mode>,
    pub monitoring: bool,
    pub cable_enabled: bool,
    pub voice_input_enabled: bool,
    pub output_holds: [bool; 3],
    pub output_labels: [String; 3],
    pub noise: bool,
    pub interaction_locked: bool,
    pub allow_mode_escape: bool,
    shortcut_labels: [String; HOTKEY_COUNT],
    shortcut_keys: [String; HOTKEY_COUNT],
    phase: String,
    message: String,
    hotkey_error: Option<String>,
    remote_hotkeys: Option<RemoteHotkeys>,
}
impl Ui {
    pub fn new(
        mode: Mode,
        hotkeys: &[Hotkey; HOTKEY_COUNT],
        overlay_mode: OverlayMode,
        startup: Option<Mode>,
        monitoring: bool,
        noise: bool,
    ) -> Result<Self, String> {
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            if RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(wide(CLASS).as_ptr()),
                hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
                ..Default::default()
            }) == 0
            {
                return Err(windows::core::Error::from_thread().to_string());
            }
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(wide(CLASS).as_ptr()),
                PCWSTR(wide(&format!("{APP_NAME} v{APP_VERSION}")).as_ptr()),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            let overlay = match Overlay::new(overlay_mode) {
                Ok(overlay) => overlay,
                Err(error) => {
                    let _ = DestroyWindow(hwnd);
                    let _ = UnregisterClassW(PCWSTR(wide(CLASS).as_ptr()), Some(instance.into()));
                    return Err(error);
                }
            };
            let icons = vec![
                mode_icon(Mode::Direct)?,
                mode_icon(Mode::Rvc)?,
                icon(0x0026b9e8, '-')?,
                icon(0x00484be2, '!')?,
                mode_icon(Mode::Muted)?,
                mode_icon(Mode::SpeakerRvc)?,
            ];
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: CALLBACK,
                hIcon: icons[2],
                ..Default::default()
            };
            let mut ui = Self {
                rvc_enabled: true,
                hwnd,
                overlay,
                data,
                icons,
                mode,
                hotkey: "None".into(),
                overlay_mode,
                startup,
                monitoring,
                cable_enabled: true,
                voice_input_enabled: false,
                output_holds: [false; 3],
                output_labels: super::config::default_output_labels(),
                noise,
                interaction_locked: false,
                allow_mode_escape: false,
                shortcut_labels: std::array::from_fn(|_| "割当なし".into()),
                shortcut_keys: std::array::from_fn(|_| "なし".into()),
                phase: "starting".into(),
                message: String::new(),
                hotkey_error: None,
                remote_hotkeys: None,
            };
            let remote_error = match RemoteHotkeys::new(hwnd, CONTROL) {
                Ok(hook) => {
                    ui.remote_hotkeys = Some(hook);
                    None
                }
                Err(error) => Some(error),
            };
            // 登録失敗でもトレイから別キーを選べるよう常駐を継続。
            if let Err(e) = ui.set_hotkeys(hotkeys) {
                ui.hotkey_error = Some(e);
            }
            if let Some(error) = remote_error {
                ui.hotkey_error = Some(error);
            }
            ui.update(mode, "starting", "音声経路を準備中")?;
            if !Shell_NotifyIconW(NIM_ADD, &ui.data).as_bool() {
                return Err("トレイの登録に失敗しました".into());
            }
            Ok(ui)
        }
    }
    pub fn set_hotkeys(&mut self, values: &[Hotkey; HOTKEY_COUNT]) -> Result<(), String> {
        self.set_hotkeys_for_rvc(values, self.rvc_enabled)
    }
    pub fn set_hotkeys_for_rvc(
        &mut self,
        values: &[Hotkey; HOTKEY_COUNT],
        rvc_enabled: bool,
    ) -> Result<(), String> {
        let bindings: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let action = HOTKEY_COMMANDS[i];
                let mut key = key.clone();
                if !rvc_enabled && matches!(action, RVC | SPK_RVC | NOISE) {
                    key.enabled = false;
                }
                (action, key)
            })
            .collect();
        self.remote_hotkeys
            .as_ref()
            .ok_or("キー検出が開始されていません")?
            .set_bindings(&bindings)?;
        self.rvc_enabled = rvc_enabled;
        self.shortcut_labels = std::array::from_fn(|i| values[i].label());
        self.shortcut_keys = std::array::from_fn(|i| values[i].key_label().into());
        self.hotkey = values[2].label();
        self.hotkey_error = None;
        Ok(())
    }
    pub fn notify(&mut self, message: &str) {
        self.hotkey_error = Some(message.into());
        let mode = self.mode;
        let phase = self.phase.clone();
        let message = self.message.clone();
        let _ = self.update(mode, &phase, &message);
    }
    pub fn update(&mut self, mode: Mode, phase: &str, message: &str) -> Result<(), String> {
        if self.mode != mode || phase != "running" {
            self.overlay.clear_telemetry();
        }
        self.mode = mode;
        self.phase = phase.into();
        self.message = message.into();
        let (index, title) = match phase {
            "running" if mode == Mode::Muted => (4, "送信停止".into()),
            "running" => (
                match mode {
                    Mode::Direct => 0,
                    Mode::SpeakerRvc => 5,
                    _ => 1,
                },
                format!("{} 出力中", mode.label()),
            ),
            "error" => (3, format!("{}：エラー・再試行待ち", mode.label())),
            "stopping" => (2, "終了しています…".into()),
            _ if mode.is_rvc() => (2, format!("{} 読込中", mode.label())),
            _ => (2, format!("{} 準備中", mode.label())),
        };
        let color = [
            0x00709b10, 0x00d75fa3, 0x0026b9e8, 0x00484be2, 0x00898076, 0x00d39530,
        ][index];
        let error = if let Some(warning) = &self.hotkey_error {
            warning.clone()
        } else if phase == "error" {
            message.into()
        } else {
            String::new()
        };
        let detail = format!(
            "MIC{}  MIC-RVC{}  SPK-RVC{}  停止{}",
            shortcut_badge(&self.shortcut_keys, DIRECT),
            shortcut_badge(&self.shortcut_keys, RVC),
            shortcut_badge(&self.shortcut_keys, SPK_RVC),
            shortcut_badge(&self.shortcut_keys, MUTED)
        );
        let noise_state = if !self.rvc_enabled {
            "RVC未設定"
        } else if !self.noise {
            "RVCノイズ抑制 OFF"
        } else {
            "RVCノイズ抑制 ON"
        };
        let state = format!(
            "表示{}{}  {}{} / {}{}{} {}{}{} {}{}{}",
            shortcut_badge(&self.shortcut_keys, OVERLAY),
            self.overlay_mode.label(),
            noise_state,
            shortcut_badge(&self.shortcut_keys, NOISE),
            self.output_labels[0],
            shortcut_badge(&self.shortcut_keys, OUTPUT1),
            output_status(self.output_holds[0], self.cable_enabled),
            self.output_labels[1],
            shortcut_badge(&self.shortcut_keys, OUTPUT2),
            output_status(self.output_holds[1], self.voice_input_enabled),
            self.output_labels[2],
            shortcut_badge(&self.shortcut_keys, MONITOR),
            output_status(self.output_holds[2], self.monitoring),
        );
        let changed = self.overlay.set_content(
            (title.clone(), state.clone(), detail, color, error),
            Buttons {
                rvc_enabled: self.rvc_enabled,
                phase,
                keys: &self.shortcut_keys,
                output_labels: &self.output_labels,
                mode,
                display: self.overlay_mode,
                outputs: [
                    self.cable_enabled,
                    self.voice_input_enabled,
                    self.monitoring,
                ],
                noise: noise_state,
                holds: self.output_holds,
            },
        );
        self.data.hIcon = self.icons[index];
        self.data.szTip.fill(0);
        let tip = wide(&format!("{title} / {state}"));
        for (to, from) in self.data.szTip.iter_mut().take(127).zip(tip) {
            *to = from;
        }
        if changed {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data);
            }
            self.overlay.refresh(&title, &state)?;
        }
        Ok(())
    }
    pub fn update_telemetry(&mut self, snapshot: Option<Snapshot>, generation: u64) {
        self.overlay
            .update_telemetry(snapshot, generation, self.mode, self.phase == "running");
    }
    pub fn set_display(
        &mut self,
        mode: OverlayMode,
        monitoring: bool,
        noise: bool,
    ) -> Result<(), String> {
        self.overlay_mode = mode;
        self.monitoring = monitoring;
        self.noise = noise;
        self.update(self.mode, &self.phase.clone(), &self.message.clone())?;
        // オフは同じ設定の再適用でも直ちに隠す。
        self.show()
    }
    pub fn show(&mut self) -> Result<(), String> {
        self.overlay.show()
    }
    unsafe fn menu(&self) -> Result<HMENU, String> {
        unsafe {
            let menu = CreatePopupMenu().map_err(|e| e.to_string())?;
            let append = |id, text: &str, checked: bool| {
                let label = shortcut_index(id).map_or_else(
                    || text.to_owned(),
                    |index| format!("{text}\t{}", self.shortcut_labels[index]),
                );
                AppendMenuW(
                    menu,
                    MF_STRING
                        | if checked { MF_CHECKED } else { MF_UNCHECKED }
                        | if self.interaction_locked && id != SETTINGS
                            || !self.rvc_enabled
                                && matches!(id, RVC | SPK_RVC | NOISE | RELEASE_RVC | 22 | 24)
                        {
                            MF_GRAYED
                        } else {
                            MF_ENABLED
                        },
                    id,
                    PCWSTR(wide(&label).as_ptr()),
                )
            };
            let result = (|| -> windows::core::Result<()> {
                append(DIRECT, "MICへ切替", self.mode == Mode::Direct)?;
                append(RVC, "MIC-RVCへ切替", self.mode == Mode::Rvc)?;
                append(SPK_RVC, "SPK-RVCへ切替", self.mode == Mode::SpeakerRvc)?;
                append(
                    MUTED,
                    "送信停止（すべての入力を送らない）",
                    self.mode == Mode::Muted,
                )?;
                append(RETRY, "現在のモードを再起動", false)?;
                append(RELEASE_RVC, "RVCを終了してMICにする", false)?;
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                append(SETTINGS, "設定…", false)?;
                append(
                    OVERLAY,
                    &format!("オーバーレイ：{}（切替）", self.overlay_mode.label()),
                    false,
                )?;
                append(
                    OUTPUT1,
                    &format!("出力1（{}）", self.output_labels[0].replace('&', "&&")),
                    self.cable_enabled,
                )?;
                append(
                    OUTPUT2,
                    &format!("出力2（{}）", self.output_labels[1].replace('&', "&&")),
                    self.voice_input_enabled,
                )?;
                append(
                    MONITOR,
                    &format!("出力3（{}）", self.output_labels[2].replace('&', "&&")),
                    self.monitoring,
                )?;
                append(NOISE, "短いノイズを抑制（RVC）", self.noise)?;
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                append(20, "ログイン時：前回のモード", self.startup.is_none())?;
                append(21, "ログイン時：MIC", self.startup == Some(Mode::Direct))?;
                append(22, "ログイン時：MIC-RVC", self.startup == Some(Mode::Rvc))?;
                append(
                    24,
                    "ログイン時：SPK-RVC",
                    self.startup == Some(Mode::SpeakerRvc),
                )?;
                append(
                    23,
                    "ログイン時：送信停止",
                    self.startup == Some(Mode::Muted),
                )?;
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                append(EXIT, "終了（音声送信を停止）", false)
            })();
            if let Err(e) = result {
                let _ = DestroyMenu(menu);
                return Err(e.to_string());
            }
            Ok(menu)
        }
    }
    pub fn poll(&mut self) -> Result<Vec<usize>, String> {
        self.overlay
            .set_interaction(self.interaction_locked, self.allow_mode_escape);
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if super::settings_ui::dialog_message(&msg) {
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let queued = std::mem::take(&mut *ACTIONS.lock().unwrap());
            let mut result = Vec::new();
            for action in queued {
                match action {
                    SHOW_MENU => {
                        let menu = self.menu()?;
                        let mut point = POINT::default();
                        let _ = GetCursorPos(&mut point);
                        let _ = SetForegroundWindow(self.hwnd);
                        let chosen = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD | TPM_RIGHTBUTTON,
                            point.x,
                            point.y,
                            None,
                            self.hwnd,
                            None,
                        )
                        .0 as usize;
                        let _ = DestroyMenu(menu);
                        let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                        if chosen != 0 {
                            result.push(chosen);
                        }
                    }
                    RESTORE => {
                        let _ = Shell_NotifyIconW(NIM_ADD, &self.data);
                    }
                    SHOW if self.interaction_locked => result.push(SETTINGS),
                    SHOW => self.show()?,
                    _ => result.push(action),
                }
            }
            self.overlay.poll_visibility();
            Ok(result)
        }
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        drop(self.remote_hotkeys.take());
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
            let _ = DestroyWindow(self.hwnd);
            for icon in &self.icons {
                let _ = DestroyIcon(*icon);
            }
            if let Ok(instance) = GetModuleHandleW(None) {
                let _ = UnregisterClassW(PCWSTR(wide(CLASS).as_ptr()), Some(instance.into()));
            }
        }
    }
}
