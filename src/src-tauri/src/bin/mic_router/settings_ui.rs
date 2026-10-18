//! 常駐専用の設定画面。保存形式と呼出しAPIを保ち、部品・フォーム・検証を分離する。
use super::config::Settings;
use audio_mixer_backend::plugins::manifest::ParameterDescriptor;
use controls::wide;
use std::sync::{
    atomic::{AtomicIsize, Ordering},
    Mutex,
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{DeleteObject, HFONT},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{DRAWITEMSTRUCT, NMHDR, TCM_GETCURSEL, TCM_SETCURSEL, TCN_SELCHANGE},
            Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT, VK_TAB},
            WindowsAndMessaging::*,
        },
    },
};

#[path = "settings_ui/controls.rs"]
mod controls;
#[path = "settings_ui/form.rs"]
mod form;
#[path = "settings_ui/help.rs"]
mod help;
#[path = "settings_ui/parameters.rs"]
mod parameters;
#[cfg(test)]
#[path = "settings_ui/tests.rs"]
mod tests;
#[path = "settings_ui/tooltip.rs"]
mod tooltip;

const CLASS: &str = "VoiceMixerMicSettingsV1";
const TAB: usize = 12;
const SAVE: usize = 1;
const CANCEL: usize = 2;
const CLIENT_WIDTH: i32 = 960;
const CLIENT_HEIGHT: i32 = 700;
const HOTKEY_BASE: usize = 1000;
use crate::actions::display_order;
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ACTIONS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_NOTIFY if l.0 != 0 => {
            let header = unsafe { &*(l.0 as *const NMHDR) };
            if header.idFrom == TAB && header.code == TCN_SELCHANGE {
                let selected =
                    unsafe { SendMessageW(header.hwndFrom, TCM_GETCURSEL, None, None).0 };
                if (0..3).contains(&selected) {
                    ACTIONS.lock().unwrap().push(10 + selected as usize);
                }
                return LRESULT(0);
            }
        }
        WM_DRAWITEM if l.0 != 0 => {
            if unsafe { help::draw_link(&*(l.0 as *const DRAWITEMSTRUCT)) } {
                return LRESULT(1);
            }
        }
        WM_SETCURSOR if matches!(unsafe { GetDlgCtrlID(HWND(w.0 as *mut _)) }, 260) => {
            unsafe {
                if let Ok(cursor) = LoadCursorW(None, IDC_HAND) {
                    SetCursor(Some(cursor));
                }
            }
            return LRESULT(1);
        }
        WM_CLOSE => {
            ACTIONS.lock().unwrap().push(CANCEL);
            return LRESULT(0);
        }
        WM_COMMAND
            if (w.0 >> 16) == 0
                || (w.0 >> 16) == CBN_SELCHANGE as usize
                || (w.0 >> 16) == EN_CHANGE as usize =>
        {
            ACTIONS.lock().unwrap().push(w.0 & 0xffff);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}
pub fn dialog_message(msg: &MSG) -> bool {
    let handle = WINDOW.load(Ordering::Relaxed);
    if handle == 0 || !(WM_KEYFIRST..=WM_KEYLAST).contains(&msg.message) {
        return false;
    }
    let window = HWND(handle as *mut _);
    if msg.message == WM_KEYDOWN
        && msg.wParam.0 == VK_TAB.0 as usize
        && unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0
        && (msg.hwnd == window || unsafe { IsChild(window, msg.hwnd) }.as_bool())
    {
        unsafe {
            if let Ok(tab) = GetDlgItem(Some(window), TAB as i32) {
                if !windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(tab).as_bool() {
                    return true;
                }
                let current = SendMessageW(tab, TCM_GETCURSEL, None, None).0.clamp(0, 2);
                let step = if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                    2
                } else {
                    1
                };
                let next = (current + step) % 3;
                SendMessageW(tab, TCM_SETCURSEL, Some(WPARAM(next as usize)), None);
                ACTIONS.lock().unwrap().push(10 + next as usize);
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(tab));
                return true;
            }
        }
    }
    // メーター等の定期通知や、他の窓・スレッド宛てのメッセージを
    // ダイアログの入力処理へ渡さない。フォーカスの移動はこの窓のキー操作だけで行う。
    unsafe {
        (msg.hwnd == window || IsChild(window, msg.hwnd).as_bool())
            && IsDialogMessageW(window, msg).as_bool()
    }
}
struct ParameterControl {
    schema: ParameterDescriptor,
    handle: HWND,
}
struct DeviceControl {
    handle: HWND,
    devices: Vec<(String, String)>,
}
pub struct SettingsWindow {
    pub hwnd: HWND,
    font: HFONT,
    link_font: HFONT,
    tips: Option<tooltip::Tooltips>,
    icon: HICON,
    base: Settings,
    pages: [Vec<HWND>; 3],
    tab: HWND,
    keys: Vec<[HWND; 7]>,
    devices: Vec<DeviceControl>,
    parameters: Vec<ParameterControl>,
    overlay: HWND,
    outputs: [HWND; 3],
    output_labels: [HWND; 3],
    shortcut_titles: Vec<HWND>,
    noise: HWND,
    rvc_enabled: HWND,
    startup: HWND,
    save: HWND,
    cancel: HWND,
    message: HWND,
    busy: bool,
}
pub enum Action {
    Save(Box<Settings>),
    Close,
}
impl Drop for SettingsWindow {
    fn drop(&mut self) {
        unsafe {
            WINDOW.store(0, Ordering::Relaxed);
            self.tips.take();
            let _ = DestroyWindow(self.hwnd);
            if !self.icon.is_invalid() {
                let _ = DestroyIcon(self.icon);
            }
            let _ = DeleteObject(self.font.into());
            let _ = DeleteObject(self.link_font.into());
            if let Ok(instance) = GetModuleHandleW(None) {
                let _ = UnregisterClassW(PCWSTR(wide(CLASS).as_ptr()), Some(instance.into()));
            }
        }
    }
}
