//! モード表示ウィンドウ。描画状態・クリック/ドラッグ・表示期限を所有する。
use super::super::config::{Mode, OverlayMode, HOTKEY_COUNT};
use super::super::telemetry::{self, Snapshot};
use super::drawing::{draw_frame, paint_buffered};
use super::{
    output_status, queue_action, wide, DIRECT, MONITOR, MUTED, NOISE, OUTPUT1, OUTPUT2, OVERLAY,
    RVC, SPK_RVC,
};
use crate::actions::shortcut_badge;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

const OVERLAY_WIDTH: i32 = 464;
const OVERLAY_HEIGHT: i32 = 130;
const INITIAL_MARGIN: i32 = 80;
pub(super) const SETTINGS_TOP: i32 = 6;
pub(super) const INPUT_TOP: i32 = 30;
pub(super) const OUTPUT_TOP: i32 = 72;
const OVERLAY_CLASS: &str = "VoiceMixerMicOverlayV1";
type Paint = (String, String, String, u32, String);

struct OverlayState {
    paint: Mutex<Paint>,
    drag: Mutex<Option<(POINT, RECT, bool, Option<usize>)>>,
    controls: Mutex<Vec<OverlayControl>>,
    locked: AtomicBool,
    allow_mode_escape: AtomicBool,
    countdown: AtomicU8,
    meters: Mutex<Option<Snapshot>>,
}
impl Default for OverlayState {
    fn default() -> Self {
        Self {
            paint: Mutex::new((
                String::new(),
                String::new(),
                String::new(),
                0,
                String::new(),
            )),
            drag: Mutex::new(None),
            controls: Mutex::new(Vec::new()),
            locked: AtomicBool::new(false),
            allow_mode_escape: AtomicBool::new(false),
            countdown: AtomicU8::new(0),
            meters: Mutex::new(None),
        }
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct OverlayControl {
    pub(super) action: usize,
    pub(super) text: String,
    pub(super) rect: RECT,
    pub(super) active: bool,
    pub(super) held: bool,
    pub(super) enabled: bool,
    pub(super) status: String,
}
const BRIEF_DURATION: Duration = Duration::from_secs(4);
// 遅延表示とIN/OUTの描画範囲。状態・入力・出力の各ボタンを含めない。
#[cfg(test)]
const TELEMETRY_RECT: RECT = RECT {
    left: 56,
    top: 52,
    right: 448,
    bottom: 126,
};
const TELEMETRY_RECTS: [RECT; 3] = [
    RECT {
        left: 56,
        top: 52,
        right: 448,
        bottom: 68,
    },
    RECT {
        left: 56,
        top: 94,
        right: 448,
        bottom: 110,
    },
    RECT {
        left: 56,
        top: 112,
        right: 448,
        bottom: 126,
    },
];
const DISPLAY_RECT: RECT = RECT {
    left: 56,
    top: SETTINGS_TOP,
    right: 250,
    bottom: SETTINGS_TOP + 21,
};
fn control_at(controls: &[OverlayControl], x: i32, y: i32) -> Option<usize> {
    controls
        .iter()
        .find(|c| {
            c.enabled
                && x >= c.rect.left
                && x < c.rect.right
                && y >= c.rect.top
                && y < c.rect.bottom
        })
        .map(|c| c.action)
}
fn mouse_point(position: LPARAM) -> POINT {
    // キャプチャ中はクライアント領域外の負座標も届く。
    POINT {
        x: position.0 as i16 as i32,
        y: (position.0 >> 16) as i16 as i32,
    }
}
fn drag_distance(start: POINT, end: POINT) -> bool {
    (end.x - start.x).abs() >= 4 || (end.y - start.y).abs() >= 4
}
fn overlay_controls(
    keys: &[String; HOTKEY_COUNT],
    mode: Mode,
    display: OverlayMode,
    outputs: [bool; 3],
    noise: &str,
    output_labels: &[String; 3],
    holds: [bool; 3],
) -> Vec<OverlayControl> {
    let mut items = Vec::new();
    for (action, label, left, right, top) in [
        (
            OVERLAY,
            format!("表示{}{}", shortcut_badge(keys, OVERLAY), display.label()),
            56,
            250,
            SETTINGS_TOP,
        ),
        (
            NOISE,
            format!("{noise}{}", shortcut_badge(keys, NOISE)),
            254,
            448,
            SETTINGS_TOP,
        ),
        (DIRECT, "MIC".into(), 56, 151, INPUT_TOP),
        (RVC, "MIC-RVC".into(), 155, 250, INPUT_TOP),
        (SPK_RVC, "SPK-RVC".into(), 254, 349, INPUT_TOP),
        (MUTED, "停止".into(), 353, 448, INPUT_TOP),
        (OUTPUT1, output_labels[0].clone(), 56, 184, OUTPUT_TOP),
        (OUTPUT2, output_labels[1].clone(), 188, 316, OUTPUT_TOP),
        (MONITOR, output_labels[2].clone(), 320, 448, OUTPUT_TOP),
    ] {
        let text = if top == SETTINGS_TOP {
            label
        } else if top == INPUT_TOP {
            format!("{label}{}", shortcut_badge(keys, action))
        } else {
            let index = match action {
                OUTPUT1 => 0,
                OUTPUT2 => 1,
                _ => 2,
            };
            format!(
                "{label}{}{}",
                shortcut_badge(keys, action),
                output_status(holds[index], outputs[index])
            )
        };
        items.push(OverlayControl {
            enabled: true,
            status: String::new(),
            action,
            text,
            held: match action {
                OUTPUT1 => holds[0],
                OUTPUT2 => holds[1],
                MONITOR => holds[2],
                _ => false,
            },
            active: match action {
                OVERLAY => display != OverlayMode::Off,
                NOISE => noise == "RVCノイズ抑制 ON",
                DIRECT => mode == Mode::Direct,
                RVC => mode == Mode::Rvc,
                SPK_RVC => mode == Mode::SpeakerRvc,
                MUTED => mode == Mode::Muted,
                OUTPUT1 => outputs[0],
                OUTPUT2 => outputs[1],
                MONITOR => outputs[2],
                _ => false,
            },
            rect: RECT {
                left,
                right,
                top,
                bottom: top + 21,
            },
        });
    }
    items
}

fn configure_inputs(controls: &mut Vec<OverlayControl>, rvc_enabled: bool, phase: &str) {
    if !rvc_enabled {
        controls.retain(|c| !matches!(c.action, RVC | SPK_RVC));
        for (action, left, right) in [(DIRECT, 56, 250), (MUTED, 254, 448)] {
            let cell = controls.iter_mut().find(|c| c.action == action).unwrap();
            cell.rect.left = left;
            cell.rect.right = right;
        }
        let noise = controls.iter_mut().find(|c| c.action == NOISE).unwrap();
        noise.enabled = false;
        noise.active = false;
    }
    for control in controls
        .iter_mut()
        .filter(|c| matches!(c.action, DIRECT | RVC | SPK_RVC))
    {
        control.status = input_status(control.active, phase).into();
    }
}

pub(super) fn input_status(active: bool, phase: &str) -> &'static str {
    if !active {
        "無効"
    } else {
        match phase {
            "running" => "",
            "error" => "エラー",
            "draining" => "送信待ち",
            "stopping" => "無効",
            _ => "ロード中",
        }
    }
}

fn brief_remaining(display: OverlayMode, elapsed: Duration) -> Option<u8> {
    (display == OverlayMode::Brief && elapsed < BRIEF_DURATION)
        .then(|| (BRIEF_DURATION.as_secs() - elapsed.as_secs()) as u8)
}

fn overlay_expired(display: OverlayMode, elapsed: Duration) -> bool {
    display == OverlayMode::Off || (display == OverlayMode::Brief && elapsed >= BRIEF_DURATION)
}

fn overlay_action_allowed_with(action: usize, locked: bool, allow_mode_escape: bool) -> bool {
    !locked || (allow_mode_escape && matches!(action, DIRECT | MUTED))
}

fn cursor_for_control(control: Option<usize>, dragging: bool) -> PCWSTR {
    if control.is_some() && !dragging {
        IDC_HAND
    } else {
        IDC_SIZEALL
    }
}

impl OverlayState {
    fn hit_control(&self, x: i32, y: i32) -> Option<usize> {
        control_at(&self.controls.lock().unwrap(), x, y)
    }
    fn overlay_action_allowed(&self, action: usize) -> bool {
        overlay_action_allowed_with(
            action,
            self.locked.load(Ordering::Relaxed),
            self.allow_mode_escape.load(Ordering::Relaxed),
        )
    }

    unsafe fn set_overlay_cursor_at(&self, point: POINT) {
        let resource = cursor_for_control(
            self.hit_control(point.x, point.y)
                .filter(|action| self.overlay_action_allowed(*action)),
            self.drag.lock().unwrap().is_some(),
        );
        if let Ok(cursor) = unsafe { LoadCursorW(None, resource) } {
            unsafe {
                SetCursor(Some(cursor));
            }
        }
    }

    unsafe fn drag_to(&self, hwnd: HWND, cursor: POINT) {
        let moved = {
            let mut drag = self.drag.lock().unwrap();
            drag.as_mut().and_then(|(start, rect, moved, _)| {
                *moved |= drag_distance(*start, cursor);
                if *moved {
                    Some((*start, *rect))
                } else {
                    None
                }
            })
        };
        if let Some((start, rect)) = moved {
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rect.left + cursor.x - start.x,
                    rect.top + cursor.y - start.y,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
                );
            }
        }
    }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(l.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        // 同期メッセージの再入を許容するため、WndProcは共有参照だけを取得する。
        // 操作中のMutexガードはSetCapture/SetWindowPos等を呼ぶ前に解放する。
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const OverlayState;
        let Some(state) = pointer.as_ref() else {
            return DefWindowProcW(hwnd, msg, w, l);
        };
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_NCHITTEST => return LRESULT(HTCLIENT as isize),
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR if l.0 as u16 as u32 == HTCLIENT => {
                let mut cursor = POINT::default();
                if GetCursorPos(&mut cursor).is_ok() && ScreenToClient(hwnd, &mut cursor).as_bool()
                {
                    state.set_overlay_cursor_at(cursor);
                    return LRESULT(1);
                }
            }
            WM_LBUTTONDOWN => {
                let point = mouse_point(l);
                let mut cursor = point;
                let mut rect = RECT::default();
                // キューを処理するまでにカーソルが動いても、実際に押した位置を使う。
                if ClientToScreen(hwnd, &mut cursor).as_bool()
                    && GetWindowRect(hwnd, &mut rect).is_ok()
                {
                    let action = state
                        .hit_control(point.x, point.y)
                        .filter(|action| state.overlay_action_allowed(*action));
                    *state.drag.lock().unwrap() = Some((cursor, rect, false, action));
                    SetCapture(hwnd);
                    state.set_overlay_cursor_at(point);
                }
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let mut cursor = POINT::default();
                if GetCursorPos(&mut cursor).is_ok() {
                    state.drag_to(hwnd, cursor);
                    if ScreenToClient(hwnd, &mut cursor).as_bool() {
                        state.set_overlay_cursor_at(cursor);
                    }
                }
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let press = state.drag.lock().unwrap().take();
                if GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                }
                if let Some((start, _, false, Some(action))) = press {
                    let point = mouse_point(l);
                    let mut cursor = point;
                    // WM_MOUSEMOVE がまとめられた場合も、離した位置から移動を検出する。
                    if ClientToScreen(hwnd, &mut cursor).as_bool()
                        && !drag_distance(start, cursor)
                        && state.hit_control(point.x, point.y) == Some(action)
                        && state.overlay_action_allowed(action)
                    {
                        queue_action(action);
                    }
                }
                state.set_overlay_cursor_at(mouse_point(l));
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            WM_CAPTURECHANGED | WM_CANCELMODE | WM_DESTROY => {
                *state.drag.lock().unwrap() = None;
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                paint_buffered(dc, rect, ps.rcPaint, |dc| {
                    draw_frame(
                        dc,
                        rect,
                        &state.paint.lock().unwrap(),
                        &state.controls.lock().unwrap(),
                        state.countdown.load(Ordering::Relaxed),
                        state.meters.lock().unwrap().as_ref(),
                    );
                });
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}

pub(super) struct Buttons<'a> {
    pub rvc_enabled: bool,
    pub phase: &'a str,
    pub keys: &'a [String; HOTKEY_COUNT],
    pub mode: Mode,
    pub display: OverlayMode,
    pub outputs: [bool; 3],
    pub holds: [bool; 3],
    pub noise: &'a str,
    pub output_labels: &'a [String; 3],
}

pub(super) struct Overlay {
    hwnd: HWND,
    state: Box<OverlayState>,
    display: OverlayMode,
    positioned: bool,
    shown: Instant,
    meter_painted: Instant,
}
impl Overlay {
    pub(super) fn new(display: OverlayMode) -> Result<Self, String> {
        let state = Box::<OverlayState>::default();
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            if RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(overlay_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(wide(OVERLAY_CLASS).as_ptr()),
                hCursor: LoadCursorW(None, IDC_SIZEALL).map_err(|e| e.to_string())?,
                ..Default::default()
            }) == 0
            {
                return Err(windows::core::Error::from_thread().to_string());
            }
            let window = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                PCWSTR(wide(OVERLAY_CLASS).as_ptr()),
                PCWSTR(wide(&crate::window_title::WindowTitle::create("")).as_ptr()),
                WS_POPUP,
                0,
                0,
                OVERLAY_WIDTH,
                OVERLAY_HEIGHT,
                None,
                None,
                Some(instance.into()),
                Some((&*state as *const OverlayState).cast()),
            );
            let hwnd = match window {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    let _ = UnregisterClassW(
                        PCWSTR(wide(OVERLAY_CLASS).as_ptr()),
                        Some(instance.into()),
                    );
                    return Err(error.to_string());
                }
            };
            let overlay = Self {
                hwnd,
                state,
                display,
                positioned: false,
                shown: Instant::now(),
                meter_painted: Instant::now() - telemetry::INTERVAL,
            };
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)
                .map_err(|e| e.to_string())?;
            Ok(overlay)
        }
    }
    pub(super) fn set_content(&mut self, content: Paint, buttons: Buttons<'_>) -> bool {
        self.display = buttons.display;
        let mut controls = overlay_controls(
            buttons.keys,
            buttons.mode,
            buttons.display,
            buttons.outputs,
            buttons.noise,
            buttons.output_labels,
            buttons.holds,
        );
        configure_inputs(&mut controls, buttons.rvc_enabled, buttons.phase);
        let controls_changed = {
            let mut current = self.state.controls.lock().unwrap();
            let changed = *current != controls;
            *current = controls;
            changed
        };
        let mut paint = self.state.paint.lock().unwrap();
        let changed = *paint != content || controls_changed;
        *paint = content;
        changed
    }
    pub(super) fn refresh(&mut self, _title: &str, _status: &str) -> Result<(), String> {
        unsafe {
            let _ = SetWindowTextW(
                self.hwnd,
                PCWSTR(wide(&crate::window_title::WindowTitle::create("")).as_ptr()),
            );
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
        self.show()
    }
    pub(super) fn clear_telemetry(&self) {
        *self.state.meters.lock().unwrap() = None;
    }
    pub(super) fn set_interaction(&self, locked: bool, allow_mode_escape: bool) {
        self.state.locked.store(locked, Ordering::Relaxed);
        self.state
            .allow_mode_escape
            .store(allow_mode_escape, Ordering::Relaxed);
    }
    pub(super) fn update_telemetry(
        &mut self,
        snapshot: Option<Snapshot>,
        generation: u64,
        mode: Mode,
        running: bool,
    ) {
        let now = Instant::now();
        if now.saturating_duration_since(self.meter_painted) < telemetry::INTERVAL {
            return;
        }
        self.meter_painted = now;
        *self.state.meters.lock().unwrap() =
            snapshot.filter(|s| running && s.fresh(now, generation, mode));
        // 表示期限やフォーカス、トレイ通知には触れず、見えている窓だけ再描画する。
        unsafe {
            if IsWindowVisible(self.hwnd).as_bool() {
                for rect in TELEMETRY_RECTS {
                    let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
                }
            }
        }
    }
    pub(super) fn show(&mut self) -> Result<(), String> {
        self.shown = Instant::now();
        self.refresh_countdown(self.shown);
        unsafe {
            if self.display == OverlayMode::Off {
                *self.state.drag.lock().unwrap() = None;
                if GetCapture() == self.hwnd {
                    let _ = ReleaseCapture();
                }
                let _ = ShowWindow(self.hwnd, SW_HIDE);
                return Ok(());
            }
            let mut area = RECT::default();
            if !self.positioned {
                SystemParametersInfoW(
                    SPI_GETWORKAREA,
                    0,
                    Some((&mut area as *mut RECT).cast()),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                )
                .map_err(|e| e.to_string())?;
            }
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                area.right - OVERLAY_WIDTH - INITIAL_MARGIN,
                area.top + INITIAL_MARGIN,
                OVERLAY_WIDTH,
                OVERLAY_HEIGHT,
                SWP_NOACTIVATE
                    | SWP_SHOWWINDOW
                    | if self.positioned {
                        SWP_NOMOVE
                    } else {
                        SET_WINDOW_POS_FLAGS(0)
                    },
            )
            .map_err(|e| e.to_string())?;
            self.positioned = true;
        }
        Ok(())
    }
    fn refresh_countdown(&self, now: Instant) {
        let remaining =
            brief_remaining(self.display, now.saturating_duration_since(self.shown)).unwrap_or(0);
        if self.state.countdown.swap(remaining, Ordering::Relaxed) != remaining {
            // 状態文字列や shown は変えず、表示ボタンの描画だけを更新する。
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), Some(&DISPLAY_RECT), false);
            }
        }
    }
    pub(super) fn poll_visibility(&mut self) {
        unsafe {
            // ドラッグ中は数秒表示を消さず、離してから表示時間を数える。
            if self.state.drag.lock().unwrap().is_some() {
                self.shown = Instant::now();
            }
            let now = Instant::now();
            self.refresh_countdown(now);
            if overlay_expired(self.display, now.saturating_duration_since(self.shown)) {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // WndProcが参照するBoxはDestroyWindow完了まで存続させる。
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            if let Ok(instance) = GetModuleHandleW(None) {
                let _ =
                    UnregisterClassW(PCWSTR(wide(OVERLAY_CLASS).as_ptr()), Some(instance.into()));
            }
        }
    }
}

#[cfg(test)]
#[path = "overlay/tests.rs"]
mod tests;
