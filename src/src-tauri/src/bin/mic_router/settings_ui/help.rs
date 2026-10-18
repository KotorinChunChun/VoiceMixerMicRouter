//! 公開リポジトリと、RVCファイルの保存フォルダーを開く。
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
    if !matches!(item.CtlID, 260) {
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

pub(super) const REPOSITORY_URL: &str = "https://github.com/KotorinChunChun/VoiceMixerMicRouter";

fn open(target: &str) -> Result<(), String> {
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(wide("open").as_ptr()),
            PCWSTR(wide(target).as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err("指定された場所を開けませんでした".into());
    }
    Ok(())
}
pub(super) fn open_repository() -> Result<(), String> {
    open(REPOSITORY_URL)
}
pub(super) fn open_folder(folder: &Path) -> Result<(), String> {
    open(&folder.to_string_lossy())
}
pub(super) fn model_folder(plugin: &Path, value: &str, default: &str) -> Result<PathBuf, String> {
    let value = value.trim().trim_matches('"');
    let file = Path::new(if value.is_empty() { default } else { value });
    let resolved = if file.is_absolute() {
        file.to_owned()
    } else {
        plugin.join(file)
    };
    let folder = resolved
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("ファイルの保存フォルダーを指定してください")?;
    if !folder.is_dir() {
        return Err(format!(
            "保存フォルダーが見つかりません: {}",
            folder.display()
        ));
    }
    Ok(folder.to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_folder_uses_selected_absolute_relative_and_empty_default_without_creating_paths() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("plugin");
        let models = plugin.join("models");
        let external = dir.path().join("任意の場所 # space");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        assert_eq!(
            model_folder(&plugin, "models/model.pth", "models/default.pth").unwrap(),
            models
        );
        assert_eq!(
            model_folder(&plugin, "", "models/default.index").unwrap(),
            models
        );
        let file = external.join("音声モデル.pth");
        assert_eq!(
            model_folder(&plugin, &format!("\"{}\"", file.display()), "").unwrap(),
            external
        );
        assert!(model_folder(&plugin, "missing/model.pth", "").is_err());
        assert!(!plugin.join("missing").exists());
    }
}
