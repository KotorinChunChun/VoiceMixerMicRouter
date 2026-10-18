//! 設定画面の標準ツールチップ。文字列とサブクラスの寿命を窓に合わせる。
use super::{controls::wide, SettingsWindow};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Graphics::Gdi::HFONT,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, WindowsAndMessaging::*},
    },
};

pub(super) const NOISE_HELP: &str = "RVCが既に保持している入力音の履歴から、前後が静かな短い衝撃音・クリック音を判定し、対応するRVC変換後の区間を小さくします。追加の音声待ちや先読みバッファは設けません。MICの音には適用しません。声と重なる音や判断に必要な後続音が足りない区間は通すため、すべてのノイズを除去する機能ではありません。";

pub(super) struct Tooltips {
    pub(super) hwnd: HWND,
    owner: HWND,
    // 各Vecの内部バッファは外側Vecが移動しても安定し、窓の破棄まで保持する。
    texts: Vec<Vec<u16>>,
    #[cfg(test)]
    pub(super) targets: Vec<HWND>,
}
impl Tooltips {
    pub(super) fn new(owner: HWND, font: HFONT) -> Result<Self, String> {
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                PCWSTR::null(),
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                Some(owner),
                None,
                Some(GetModuleHandleW(None).map_err(|e| e.to_string())?.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            let tips = Self {
                hwnd,
                owner,
                texts: Vec::new(),
                #[cfg(test)]
                targets: Vec::new(),
            };
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(|e| e.to_string())?;
            SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(font.0 as usize)), None);
            SendMessageW(hwnd, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(560)));
            SendMessageW(
                hwnd,
                TTM_SETDELAYTIME,
                Some(WPARAM(TTDT_INITIAL as usize)),
                Some(LPARAM(500)),
            );
            SendMessageW(
                hwnd,
                TTM_SETDELAYTIME,
                Some(WPARAM(TTDT_AUTOPOP as usize)),
                Some(LPARAM(30000)),
            );
            Ok(tips)
        }
    }
    pub(super) fn add(&mut self, target: HWND, text: &str) -> Result<(), String> {
        let mut buffer = wide(text);
        let info = TTTOOLINFOW {
            // v6の有効化に依存しない標準窓ではV2サイズで登録する。
            cbSize: std::mem::offset_of!(TTTOOLINFOW, lpReserved) as u32,
            uFlags: TTF_IDISHWND | TTF_SUBCLASS,
            hwnd: self.owner,
            uId: target.0 as usize,
            lpszText: PWSTR(buffer.as_mut_ptr()),
            ..Default::default()
        };
        let added = unsafe {
            SendMessageW(
                self.hwnd,
                TTM_ADDTOOLW,
                None,
                Some(LPARAM(&info as *const _ as isize)),
            )
        };
        if added.0 == 0 {
            return Err("ツールチップを登録できません".into());
        }
        self.texts.push(buffer);
        #[cfg(test)]
        self.targets.push(target);
        Ok(())
    }
    pub(super) fn pop(&self) {
        unsafe {
            SendMessageW(self.hwnd, TTM_POP, None, None);
        }
    }
}
impl Drop for Tooltips {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
impl SettingsWindow {
    pub(super) fn tip(&mut self, target: HWND, text: &str) -> Result<(), String> {
        self.tips
            .as_mut()
            .ok_or("ツールチップが初期化されていません")?
            .add(target, text)
    }
}
