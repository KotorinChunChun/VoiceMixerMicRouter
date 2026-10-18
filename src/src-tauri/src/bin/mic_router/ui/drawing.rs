//! オーバーレイの描画とGDI資源。表示・操作状態は保持しない。
use super::super::telemetry::{self, Snapshot};
use super::overlay::{OverlayControl, INPUT_TOP, OUTPUT_TOP, SETTINGS_TOP};
use super::{wide, DIRECT, MONITOR, MUTED, NOISE, OUTPUT1, OUTPUT2, OVERLAY, RVC, SPK_RVC};
use std::time::Instant;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{COLORREF, RECT, SIZE},
        Graphics::Gdi::*,
    },
};

pub(super) const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}

pub(super) fn control_colors(control: &OverlayControl) -> (COLORREF, COLORREF, COLORREF) {
    if !control.active {
        return (rgb(52, 59, 70), rgb(87, 96, 109), rgb(203, 210, 220));
    }
    if control.held {
        return (rgb(36, 132, 205), rgb(139, 216, 255), rgb(255, 255, 255));
    }
    let (background, border) = match control.action {
        DIRECT => (rgb(76, 139, 49), rgb(159, 223, 126)),
        RVC => (rgb(134, 96, 183), rgb(211, 173, 248)),
        SPK_RVC => (rgb(55, 124, 171), rgb(138, 204, 248)),
        MUTED => (rgb(105, 120, 135), rgb(200, 214, 228)),
        NOISE => (rgb(145, 113, 57), rgb(237, 199, 124)),
        OVERLAY => (rgb(50, 109, 145), rgb(137, 195, 229)),
        _ => (rgb(39, 134, 124), rgb(129, 226, 209)),
    };
    (background, border, rgb(255, 255, 255))
}

pub(super) fn control_text(control: &OverlayControl, countdown: u8) -> String {
    if control.action == OVERLAY && countdown != 0 {
        format!("{} {countdown}秒", control.text)
    } else {
        control.text.clone()
    }
}

/// 用途名だけを省略し、キーとON/OFFを残す。文字数ではなく現在のフォント幅で判断する。
pub(super) fn fit_output_text(text: &str, width: i32, measure: impl Fn(&str) -> i32) -> String {
    if measure(text) <= width {
        return text.into();
    }
    let (label, suffix) = if let Some((label, suffix)) = text.rsplit_once('[') {
        (label, format!("[{suffix}"))
    } else if let Some(label) = text.strip_suffix("P2T") {
        (label, "P2T".into())
    } else if let Some(label) = text.strip_suffix("OFF") {
        (label, "OFF".into())
    } else if let Some(label) = text.strip_suffix("ON") {
        (label, "ON".into())
    } else {
        return text.into();
    };
    let mut label = label.to_owned();
    while !label.is_empty() {
        label.pop();
        let shortened = format!("{label}…{suffix}");
        if measure(&shortened) <= width {
            return shortened;
        }
    }
    suffix
}

pub(super) unsafe fn overlay_control_font() -> HFONT {
    unsafe {
        CreateFontW(
            -12,
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
        )
    }
}

pub(super) unsafe fn draw_control(dc: HDC, control: &OverlayControl, countdown: u8) {
    let (background, border, foreground) = control_colors(control);
    unsafe {
        let brush = CreateSolidBrush(background);
        FillRect(dc, &control.rect, brush);
        let _ = DeleteObject(brush.into());
        let brush = CreateSolidBrush(border);
        FrameRect(dc, &control.rect, brush);
        let _ = DeleteObject(brush.into());
        SetTextColor(dc, foreground);
        let mut area = control.rect;
        area.left += 2;
        area.right -= 2;
        let mut label = control_text(control, countdown);
        if matches!(control.action, OUTPUT1 | OUTPUT2 | MONITOR) {
            label = fit_output_text(&label, area.right - area.left, |value| {
                let chars: Vec<_> = value.encode_utf16().collect();
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &chars, &mut size);
                size.cx
            });
        }
        let mut text: Vec<u16> = label.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut text,
            &mut area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

struct OverlayBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl OverlayBuffer {
    unsafe fn new(target: HDC, width: i32, height: i32) -> Option<Self> {
        unsafe {
            let dc = CreateCompatibleDC(Some(target));
            if dc.0.is_null() {
                return None;
            }
            let bitmap = CreateCompatibleBitmap(target, width, height);
            if bitmap.0.is_null() {
                let _ = DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap.into());
            if previous.0.is_null() || previous.0 as isize == -1 {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(dc);
                return None;
            }
            Some(Self {
                dc,
                bitmap,
                previous,
            })
        }
    }
}
impl Drop for OverlayBuffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
pub(super) unsafe fn paint_buffered(
    target: HDC,
    bounds: RECT,
    dirty: RECT,
    draw: impl FnOnce(HDC),
) {
    unsafe {
        if bounds.right <= 0 || bounds.bottom <= 0 {
            return;
        }
        if let Some(buffer) = OverlayBuffer::new(target, bounds.right, bounds.bottom) {
            draw(buffer.dc);
            // 背景だけの途中状態を画面へ出さず、完成した更新領域だけ転送する。
            let _ = BitBlt(
                target,
                dirty.left,
                dirty.top,
                dirty.right - dirty.left,
                dirty.bottom - dirty.top,
                Some(buffer.dc),
                dirty.left,
                dirty.top,
                SRCCOPY,
            );
        } else {
            // GDIリソース不足時も、表示自体は維持する。
            draw(target);
        }
    }
}

pub(super) unsafe fn draw_frame(
    dc: HDC,
    rect: RECT,
    content: &(String, String, String, u32, String),
    controls: &[OverlayControl],
    countdown: u8,
    snapshot: Option<&Snapshot>,
) {
    unsafe {
        fill(dc, rect, COLORREF(0x0028201c));
        fill(
            dc,
            RECT {
                left: 0,
                top: 0,
                right: 6,
                bottom: rect.bottom,
            },
            COLORREF(content.3),
        );
        SetBkMode(dc, TRANSPARENT);
        let font = overlay_control_font();
        let old = SelectObject(dc, font.into());
        for (label, top) in [
            ("設定", SETTINGS_TOP),
            ("入力", INPUT_TOP),
            ("出力", OUTPUT_TOP),
        ] {
            label_at(
                dc,
                label,
                RECT {
                    left: 18,
                    top,
                    right: 52,
                    bottom: top + 21,
                },
                false,
            );
        }
        for control in controls {
            draw_control(dc, control, countdown);
        }
        draw_meters(dc, controls, snapshot);
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
    }
}

unsafe fn fill(dc: HDC, rect: RECT, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

unsafe fn label_at(dc: HDC, value: &str, mut rect: RECT, right: bool) {
    unsafe {
        SetTextColor(dc, COLORREF(0x00cbc6c1));
        let mut text: Vec<_> = value.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut text,
            &mut rect,
            (if right { DT_RIGHT } else { DT_CENTER })
                | DT_VCENTER
                | DT_SINGLELINE
                | DT_END_ELLIPSIS
                | DT_NOPREFIX,
        );
    }
}

/// 約1秒の固定時間軸。無効・準備・障害はバーと同じ矩形に表示する。
unsafe fn draw_meters(dc: HDC, controls: &[OverlayControl], snapshot: Option<&Snapshot>) {
    let now = Instant::now();
    let snapshot = snapshot.filter(|s| now.saturating_duration_since(s.at) <= telemetry::STALE);
    let input = snapshot.map(|s| s.bins(now)[0]);
    let outputs = snapshot.map(|s| s.output_bins(now));
    unsafe {
        for control in controls {
            let output_index = match control.action {
                OUTPUT1 => Some(0),
                OUTPUT2 => Some(1),
                MONITOR => Some(2),
                _ => None,
            };
            let is_input = matches!(control.action, DIRECT | RVC | SPK_RVC);
            if !is_input && output_index.is_none() {
                continue;
            }
            let top = control.rect.bottom + 1;
            let area = RECT {
                left: control.rect.left,
                top,
                right: control.rect.right,
                bottom: top + 16,
            };
            let status = if is_input {
                control.status.as_str()
            } else if !control.active
                || snapshot.is_some_and(|s| !s.outputs_enabled[output_index.unwrap()])
            {
                "無効"
            } else {
                ""
            };
            if !status.is_empty() {
                label_at(dc, status, area, false);
                continue;
            }
            let bins = if let Some(index) = output_index {
                outputs.map(|rows| rows[index])
            } else {
                input
            };
            draw_history(dc, area, bins.as_ref(), is_input);
        }
        let selected_rvc = controls
            .iter()
            .any(|c| c.active && matches!(c.action, RVC | SPK_RVC));
        if selected_rvc {
            let text = snapshot.and_then(|s| s.latency_ms).map_or_else(
                || "遅延 測定待ち".into(),
                |ms| format!("遅延 約{:.0}ms", (ms / 10.0).round() * 10.0),
            );
            // 遅延だけは補助表示として右下に置く。
            let font = CreateFontW(
                -10,
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
            let old = SelectObject(dc, font.into());
            label_at(
                dc,
                &text,
                RECT {
                    left: 254,
                    top: 112,
                    right: 448,
                    bottom: 126,
                },
                true,
            );
            SelectObject(dc, old);
            let _ = DeleteObject(font.into());
        }
    }
}

unsafe fn draw_history(
    dc: HDC,
    area: RECT,
    bins: Option<&[Option<f32>; telemetry::BINS]>,
    input: bool,
) {
    unsafe {
        fill(dc, area, COLORREF(0x003b322d));
        let Some(bins) = bins else {
            return;
        };
        let width = area.right - area.left;
        for (i, peak) in bins.iter().enumerate() {
            let Some(peak) = peak else {
                continue;
            };
            let height = (telemetry::level_height(*peak) * 16.0).round() as i32;
            if height == 0 {
                continue;
            }
            let color = if *peak >= 1.0 {
                0x006666f5
            } else if input {
                0x008fd755
            } else {
                0x00e3aa73
            };
            fill(
                dc,
                RECT {
                    left: area.left + i as i32 * width / telemetry::BINS as i32,
                    top: area.bottom - height,
                    right: area.left + (i + 1) as i32 * width / telemetry::BINS as i32 - 1,
                    bottom: area.bottom,
                },
                COLORREF(color),
            );
        }
    }
}
