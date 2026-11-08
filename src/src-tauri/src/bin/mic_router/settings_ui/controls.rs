//! 設定画面で使うWin32部品の生成と値の読み書き。
use super::SettingsWindow;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
};

pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub(super) fn text(hwnd: HWND) -> String {
    unsafe {
        let mut buf = vec![0u16; GetWindowTextLengthW(hwnd) as usize + 1];
        let len = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..len as usize])
    }
}
pub(super) fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(text).as_ptr()));
    }
}
pub(super) fn checked(hwnd: HWND) -> bool {
    unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None).0 == 1 }
}
pub(super) fn set_checked(hwnd: HWND, on: bool) {
    unsafe {
        SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(on as usize)), None);
    }
}
pub(super) fn selection(hwnd: HWND) -> usize {
    unsafe { SendMessageW(hwnd, CB_GETCURSEL, None, None).0 as usize }
}
pub(super) fn select(hwnd: HWND, index: usize) {
    unsafe {
        SendMessageW(hwnd, CB_SETCURSEL, Some(WPARAM(index)), None);
    }
}
pub(super) fn select_label(hwnd: HWND, label: &str) {
    let index = unsafe {
        SendMessageW(
            hwnd,
            CB_FINDSTRINGEXACT,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(wide(label).as_ptr() as isize)),
        )
        .0
    };
    if index >= 0 {
        select(hwnd, index as usize);
    }
}

// 初回表示では現在のキーだけを入れ、一覧が必要になった行で全候補を用意する。
unsafe extern "system" fn primary_key_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if message == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(primary_key_proc), id);
        } else if matches!(
            message,
            WM_SETFOCUS | WM_LBUTTONDOWN | WM_KEYDOWN | WM_MOUSEWHEEL
                | CB_SHOWDROPDOWN | CB_GETCOUNT | CB_GETLBTEXT | CB_GETLBTEXTLEN
                | CB_FINDSTRING | CB_FINDSTRINGEXACT | CB_SETCURSEL
        ) {
            let _ = RemoveWindowSubclass(hwnd, Some(primary_key_proc), id);
            let current = text(hwnd);
            let choices: Vec<String> = std::iter::once("なし".into())
                .chain((b'A'..=b'Z').map(|c| (c as char).to_string()))
                .chain((1..=24).map(|n| format!("F{n}")))
                .collect();
            SendMessageW(hwnd, CB_RESETCONTENT, None, None);
            add_combo_items(hwnd, &choices);
            select_label(hwnd, &current);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

fn add_combo_items(hwnd: HWND, values: &[String]) {
    unsafe {
        for value in values {
            SendMessageW(
                hwnd,
                CB_ADDSTRING,
                None,
                Some(LPARAM(wide(value).as_ptr() as isize)),
            );
        }
    }
}

impl SettingsWindow {
    pub(super) fn primary_key_combo(
        &mut self,
        value: &str,
        x: i32,
        y: i32,
        width: i32,
        id: usize,
    ) -> Result<HWND, String> {
        let mut initial = vec!["なし".into()];
        if value != "なし" {
            initial.push(value.into());
        }
        let hwnd = self.combo(&initial, value, false, x, y, width, id, 1)?;
        if !unsafe { SetWindowSubclass(hwnd, Some(primary_key_proc), 0, 0) }.as_bool() {
            return Err("主キーの候補を初期化できません".into());
        }
        Ok(hwnd)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn control(
        &mut self,
        class: &str,
        label: &str,
        style: u32,
        id: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        page: Option<usize>,
    ) -> Result<HWND, String> {
        unsafe {
            let handle = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(wide(class).as_ptr()),
                PCWSTR(wide(label).as_ptr()),
                WS_CHILD
                    | if page.is_none_or(|index| index == self.active_page.get()) {
                        WS_VISIBLE
                    } else {
                        WINDOW_STYLE(0)
                    }
                    | WINDOW_STYLE(style)
                    | if class == "STATIC" {
                        WINDOW_STYLE(0)
                    } else {
                        WS_TABSTOP
                    },
                x,
                y,
                w,
                h,
                Some(self.hwnd),
                Some(HMENU(id as *mut _)),
                Some(GetModuleHandleW(None).map_err(|e| e.to_string())?.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            SendMessageW(
                handle,
                WM_SETFONT,
                Some(WPARAM(self.font.0 as usize)),
                Some(LPARAM(1)),
            );
            if let Some(page) = page {
                self.pages[page].push(handle);
            }
            Ok(handle)
        }
    }
    pub(super) fn label(
        &mut self,
        label: &str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        page: usize,
    ) -> Result<HWND, String> {
        self.control(
            "STATIC",
            label,
            0x4000 | 0x0080 | 0x0100, /* SS_ENDELLIPSIS | SS_NOPREFIX | SS_NOTIFY */
            0,
            x,
            y,
            w,
            h,
            Some(page),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn checkbox(
        &mut self,
        label: &str,
        on: bool,
        x: i32,
        y: i32,
        w: i32,
        id: usize,
        page: usize,
    ) -> Result<HWND, String> {
        let h = self.control(
            "BUTTON",
            label,
            BS_AUTOCHECKBOX as u32,
            id,
            x,
            y,
            w,
            28,
            Some(page),
        )?;
        set_checked(h, on);
        Ok(h)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn combo(
        &mut self,
        values: &[String],
        value: &str,
        editable: bool,
        x: i32,
        y: i32,
        w: i32,
        id: usize,
        page: usize,
    ) -> Result<HWND, String> {
        let h = self.control(
            "COMBOBOX",
            "",
            WS_VSCROLL.0
                | if editable {
                    CBS_DROPDOWN as u32 | CBS_AUTOHSCROLL as u32
                } else {
                    CBS_DROPDOWNLIST as u32
                },
            id,
            x,
            y,
            w,
            260,
            Some(page),
        )?;
        if editable {
            unsafe {
                SendMessageW(h, CB_LIMITTEXT, Some(WPARAM(32767)), None);
            }
        }
        add_combo_items(h, values);
        if let Some(index) = values.iter().position(|s| s == value) {
            select(h, index);
        } else if editable {
            set_text(h, value);
        }
        Ok(h)
    }
}
