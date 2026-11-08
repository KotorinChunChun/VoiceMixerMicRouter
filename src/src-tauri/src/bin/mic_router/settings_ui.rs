//! 常駐専用の設定画面。保存形式と呼出しAPIを保ち、部品・フォーム・検証を分離する。
use super::config::Settings;
use audio_mixer_backend::plugins::manifest::ParameterDescriptor;
use controls::wide;
use std::{
    cell::Cell,
    sync::{
        atomic::{AtomicIsize, Ordering},
        Mutex,
    },
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{
                DRAWITEMSTRUCT, MEASUREITEMSTRUCT, NMHDR, ODS_FOCUS, TCHITTESTINFO, TCM_GETCURSEL,
                TCM_HITTEST, TCM_SETCURSEL, TCN_SELCHANGE,
            },
            Input::KeyboardAndMouse::{
                GetKeyState, IsWindowEnabled, VK_CONTROL, VK_LEFT, VK_RIGHT, VK_SHIFT, VK_TAB,
            },
            Shell::{DefSubclassProc, RemoveWindowSubclass},
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
const RESET_KEYS: usize = 3;
const RESET_SOUNDS: usize = 4;
const SHOW_UNSELECTED: usize = 5;
const RESET_BASIC: usize = 6;
const RESET_RVC: usize = 7;
const RVC_ENABLED: usize = 222;
const RVC_PAGE: usize = 2;
const TAB_TITLES: [&str; 4] = ["基本設定", "キー設定", "RVC設定", "効果音"];
const TAB_WIDTH: u32 = 145;
const TAB_HEIGHT: u32 = 32;
const CLIENT_WIDTH: i32 = 960;
const CLIENT_HEIGHT: i32 = 700;
const HOTKEY_BASE: usize = 1000;
use crate::actions::display_order;
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ACTIONS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

fn rvc_tab_available(window: HWND) -> bool {
    unsafe { GetDlgItem(Some(window), RVC_ENABLED as i32) }.is_ok_and(controls::checked)
}

fn next_page(current: usize, backwards: bool, rvc_available: bool) -> usize {
    let step = if backwards { TAB_TITLES.len() - 1 } else { 1 };
    let next = (current + step) % TAB_TITLES.len();
    if next == RVC_PAGE && !rvc_available {
        (next + step) % TAB_TITLES.len()
    } else {
        next
    }
}

unsafe extern "system" fn tab_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    parent: usize,
) -> LRESULT {
    unsafe {
        let window = HWND(parent as *mut _);
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(tab_proc), id);
        } else if !rvc_tab_available(window) {
            if matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) {
                let mut hit = TCHITTESTINFO::default();
                hit.pt.x = l.0 as i16 as i32;
                hit.pt.y = (l.0 >> 16) as i16 as i32;
                if SendMessageW(
                    hwnd,
                    TCM_HITTEST,
                    None,
                    Some(LPARAM(&mut hit as *mut _ as isize)),
                )
                .0 == RVC_PAGE as isize
                {
                    return LRESULT(0);
                }
            }
            if msg == WM_KEYDOWN && (w.0 == VK_LEFT.0 as usize || w.0 == VK_RIGHT.0 as usize) {
                let current = SendMessageW(hwnd, TCM_GETCURSEL, None, None).0;
                let backwards = w.0 == VK_LEFT.0 as usize;
                if (current == 1 && !backwards) || (current == 3 && backwards) {
                    let next = next_page(current as usize, backwards, false);
                    SendMessageW(hwnd, TCM_SETCURSEL, Some(WPARAM(next)), None);
                    ACTIONS.lock().unwrap().push(10 + next);
                    return LRESULT(0);
                }
            }
        }
        DefSubclassProc(hwnd, msg, w, l)
    }
}

unsafe fn draw_tab(window: HWND, item: &DRAWITEMSTRUCT) {
    unsafe {
        let Some(title) = TAB_TITLES.get(item.itemID as usize) else {
            return;
        };
        let dc = item.hDC;
        FillRect(dc, &item.rcItem, GetSysColorBrush(COLOR_BTNFACE));
        let font = SendMessageW(item.hwndItem, WM_GETFONT, None, None);
        let old_font = SelectObject(dc, HFONT(font.0 as *mut _).into());
        SetBkMode(dc, TRANSPARENT);
        let color = if !IsWindowEnabled(item.hwndItem).as_bool()
            || item.itemID as usize == RVC_PAGE && !rvc_tab_available(window)
        {
            COLOR_GRAYTEXT
        } else {
            COLOR_BTNTEXT
        };
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(GetSysColor(color)));
        let mut area = item.rcItem;
        let mut label: Vec<_> = title.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut label,
            &mut area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        if item.itemState.0 & ODS_FOCUS.0 != 0 {
            let _ = DrawFocusRect(dc, &item.rcItem);
        }
        SelectObject(dc, old_font);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_NOTIFY if l.0 != 0 => {
            let header = unsafe { &*(l.0 as *const NMHDR) };
            if header.idFrom == TAB && header.code == TCN_SELCHANGE {
                let selected =
                    unsafe { SendMessageW(header.hwndFrom, TCM_GETCURSEL, None, None).0 };
                if (0..4).contains(&selected) {
                    ACTIONS.lock().unwrap().push(10 + selected as usize);
                }
                return LRESULT(0);
            }
        }
        WM_MEASUREITEM if w.0 == TAB && l.0 != 0 => {
            let item = unsafe { &mut *(l.0 as *mut MEASUREITEMSTRUCT) };
            item.itemWidth = TAB_WIDTH;
            item.itemHeight = TAB_HEIGHT;
            return LRESULT(1);
        }
        WM_DRAWITEM if l.0 != 0 => {
            let item = unsafe { &*(l.0 as *const DRAWITEMSTRUCT) };
            if item.CtlID as usize == TAB {
                unsafe { draw_tab(hwnd, item) };
                return LRESULT(1);
            }
            if unsafe { help::draw_link(item) } {
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
                if !IsWindowEnabled(tab).as_bool() {
                    return true;
                }
                let current = SendMessageW(tab, TCM_GETCURSEL, None, None).0.clamp(0, 3);
                let backwards = GetKeyState(VK_SHIFT.0 as i32) < 0;
                let next = next_page(current as usize, backwards, rvc_tab_available(window));
                SendMessageW(tab, TCM_SETCURSEL, Some(WPARAM(next)), None);
                ACTIONS.lock().unwrap().push(10 + next);
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
    original_input: (String, bool),
}
struct DeviceControl {
    handle: HWND,
    devices: Vec<(String, String)>,
    disconnected_choice: Option<usize>,
}
pub struct SettingsWindow {
    pub hwnd: HWND,
    font: HFONT,
    link_font: HFONT,
    tips: Option<tooltip::Tooltips>,
    icon: HICON,
    base: Settings,
    pages: [Vec<HWND>; 4],
    tab: HWND,
    active_page: Cell<usize>,
    keys: Vec<[HWND; 7]>,
    devices: Vec<DeviceControl>,
    device_rows: [Vec<HWND>; 5],
    show_unselected: bool,
    expand_devices: HWND,
    parameters: Vec<ParameterControl>,
    sounds: [HWND; 7],
    sound_device: Option<DeviceControl>,
    overlay: HWND,
    outputs: [HWND; 3],
    output_labels: [HWND; 3],
    shortcut_titles: Vec<HWND>,
    noise: HWND,
    rvc_enabled: HWND,
    startup: HWND,
    startup_modes: Vec<Option<super::config::Mode>>,
    save: HWND,
    cancel: HWND,
    message: HWND,
    busy: bool,
    rvc_reset: bool,
}
pub enum Action {
    Save(Box<Settings>),
    Close,
    Preview(u8, String),
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
