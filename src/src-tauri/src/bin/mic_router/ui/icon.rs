//! トレイと設定画面で共有する、正立したモードアイコン。
use super::super::config::Mode;
use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, HICON};

pub(super) fn icon(color: u32, letter: char) -> Result<HICON, String> {
    let glyph: [u8; 7] = match letter {
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        _ => [0, 0, 0, 31, 0, 0, 0],
    };
    let mut pixels = vec![0u8; 32 * 32 * 4];
    for y in 0usize..32 {
        for x in 0usize..32 {
            let gx = x.checked_sub(6usize).map(|v| v / 4);
            let gy = y.checked_sub(2usize).map(|v| v / 4);
            let white = match (gx, gy) {
                (Some(gx), Some(gy)) if gx < 5 && gy < 7 => glyph[gy] & (1 << (4 - gx)) != 0,
                _ => false,
            };
            let c = if white { 0xffffff } else { color };
            // CreateIconには表示順のBGRAを渡す。DIBの下から上への反転を加えない。
            let p = (y * 32 + x) * 4;
            pixels[p..p + 4].copy_from_slice(&[
                ((c >> 16) & 255) as u8,
                ((c >> 8) & 255) as u8,
                (c & 255) as u8,
                255,
            ]);
        }
    }
    unsafe {
        CreateIcon(None, 32, 32, 1, 32, [0u8; 128].as_ptr(), pixels.as_ptr())
            .map_err(|e| e.to_string())
    }
}

pub fn mode_icon(mode: Mode) -> Result<HICON, String> {
    match mode {
        Mode::Direct => icon(0x00709b10, 'M'),
        Mode::Rvc => icon(0x00d75fa3, 'R'),
        Mode::SpeakerRvc => icon(0x00d39530, 'S'),
        Mode::Muted => icon(0x00898076, 'X'),
    }
}
