//! 設定画面の2リンクから、同梱されたローカル案内の該当節を開く。
use super::controls::wide;
use std::path::{Path, PathBuf};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::COLORREF,
        Graphics::Gdi::*,
        UI::{
            Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS},
            Shell::ShellExecuteW,
            WindowsAndMessaging::*,
        },
    },
};

/// comctl32 v6に依存せず、標準Buttonのキー操作・アクセシビリティを保つ。
pub(super) unsafe fn draw_link(item: &DRAWITEMSTRUCT) -> bool {
    if !matches!(item.CtlID, 260 | 261) {
        return false;
    }
    unsafe {
        FillRect(item.hDC, &item.rcItem, GetSysColorBrush(COLOR_BTNFACE));
        let font = SendMessageW(item.hwndItem, WM_GETFONT, None, None);
        let old = SelectObject(item.hDC, HFONT(font.0 as *mut _).into());
        SetBkMode(item.hDC, TRANSPARENT);
        SetTextColor(
            item.hDC,
            if item.itemState.0 & ODS_DISABLED.0 != 0 {
                COLORREF(GetSysColor(COLOR_GRAYTEXT))
            } else {
                COLORREF(0x00ba5c07)
            },
        );
        let mut area = item.rcItem;
        let mut text: Vec<_> = super::controls::text(item.hwndItem)
            .encode_utf16()
            .collect();
        DrawTextW(
            item.hDC,
            &mut text,
            &mut area,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        if item.itemState.0 & ODS_FOCUS.0 != 0 {
            let _ = DrawFocusRect(item.hDC, &item.rcItem);
        }
        SelectObject(item.hDC, old);
    }
    true
}

fn target(action: usize, executable: &Path) -> Result<String, String> {
    let section = match action {
        260 => "install",
        261 => "settings",
        _ => return Err("案内が見つかりません".into()),
    };
    let folder = executable.parent().ok_or("アプリの配置先がありません")?;
    let file: PathBuf = folder.join("browser-capture-guide.html");
    // デバッグ単体実行でも案内を利用できるよう、同じ内容を配置する。
    if !file.exists() {
        std::fs::write(
            &file,
            include_str!("../../../../../../docs/browser-capture-guide.html"),
        )
        .map_err(|e| format!("案内を配置できません: {e}"))?;
    }
    let path = file.to_string_lossy().replace('\\', "/");
    let mut url = String::from("file:///");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/:.-_~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(format!("{url}#{section}"))
}
pub(super) fn open(action: usize) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let url = target(action, &executable)?;
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(wide("open").as_ptr()),
            PCWSTR(wide(&url).as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err("ブラウザで案内を開けませんでした".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_guides_are_available_with_encoded_paths_and_correct_sections() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("日本語 # space");
        std::fs::create_dir(&folder).unwrap();
        let exe = folder.join("mic.exe");
        let install = target(260, &exe).unwrap();
        assert!(install.contains("%23%20space"));
        assert!(install.ends_with("#install"));
        assert!(target(261, &exe).unwrap().ends_with("#settings"));
        let html = std::fs::read_to_string(folder.join("browser-capture-guide.html")).unwrap();
        assert!(html.contains("id=\"install\"") && html.contains("id=\"settings\""));
        assert!(html.contains("直接受信しません"));
        assert!(target(0, &exe).is_err());
    }
}
