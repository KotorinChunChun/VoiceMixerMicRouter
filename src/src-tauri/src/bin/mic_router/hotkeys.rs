//! 全アプリとMSTSCの押下・解放を観測し、割当ごとに共存/上書きを適用する。
use super::config::{Hotkey, KeyHandling};
use crate::actions::{ptt_event, PTT_RELEASE_OFFSET};
use std::{
    cell::RefCell,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{
                GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{Input::KeyboardAndMouse::GetAsyncKeyState, WindowsAndMessaging::*},
    },
};
const WAKE: u32 = WM_APP + 176;
const MODIFIER_KEYS: [usize; 8] = [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c];
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    action: usize,
    modifiers: u32,
    vk: usize,
    handling: KeyHandling,
}
fn bindings(values: &[(usize, Hotkey)]) -> Result<Vec<Binding>, String> {
    let mut result: Vec<Binding> = Vec::new();
    for (action, key) in values.iter().filter(|(_, k)| k.is_assigned()) {
        let vk = key.virtual_key()? as usize;
        let modifiers = key.modifiers();
        if result
            .iter()
            .any(|b| b.vk == vk && b.modifiers == modifiers)
        {
            return Err("同じキーを複数の操作に割り当てられません".into());
        }
        result.push(Binding {
            action: *action,
            modifiers,
            vk,
            handling: key.handling,
        });
    }
    Ok(result)
}
struct KeyState {
    bindings: Vec<Binding>,
    down: [bool; 256],
    active: Vec<Binding>,
    suppressed: [bool; 256],
}
impl KeyState {
    fn new() -> Self {
        Self {
            bindings: vec![],
            down: [false; 256],
            active: vec![],
            suppressed: [false; 256],
        }
    }
    fn modifiers(&self) -> u32 {
        (self.down[0xa4] || self.down[0xa5]) as u32
            | (((self.down[0xa2] || self.down[0xa3]) as u32) << 1)
            | (((self.down[0xa0] || self.down[0xa1]) as u32) << 2)
            | (((self.down[0x5b] || self.down[0x5c]) as u32) << 3)
    }
    fn event(&mut self, vk: usize, pressed: bool) -> Vec<usize> {
        if vk >= 256 {
            return vec![];
        }
        let relevant = MODIFIER_KEYS.contains(&vk)
            || self.bindings.iter().any(|b| b.vk == vk)
            || self.active.iter().any(|b| b.vk == vk);
        if !relevant {
            return vec![];
        }
        let repeated = self.down[vk];
        self.down[vk] = pressed;
        let modifiers = self.modifiers();
        let mut events = vec![];
        self.active.retain(|b| {
            let keep = self.down[b.vk] && (modifiers & b.modifiers) == b.modifiers;
            if !keep {
                events.push(b.action + PTT_RELEASE_OFFSET)
            }
            keep
        });
        if pressed && !repeated {
            if let Some(b) = self
                .bindings
                .iter()
                .find(|b| b.vk == vk && b.modifiers == modifiers)
                .cloned()
            {
                events.push(b.action);
                if ptt_event(b.action).is_some() {
                    self.active.push(b)
                }
            }
        }
        events
    }
    fn observe(&mut self, vk: usize, pressed: bool) -> (Vec<usize>, bool) {
        if vk >= 256 {
            return (vec![], false);
        }
        let matched = pressed
            && !self.down[vk]
            && self.bindings.iter().any(|b| {
                b.vk == vk && b.modifiers == self.modifiers() && b.handling == KeyHandling::Override
            });
        let suppress = self.suppressed[vk] || matched;
        let events = self.event(vk, pressed);
        // 割当変更・修飾キー解放後も、握り潰した主キーのkeyupまで対にする。
        self.suppressed[vk] = pressed && suppress;
        (events, suppress)
    }
    fn replace_bindings(
        &mut self,
        values: Vec<Binding>,
        mut is_down: impl FnMut(usize) -> bool,
    ) -> Vec<usize> {
        let released = self
            .active
            .drain(..)
            .map(|b| b.action + PTT_RELEASE_OFFSET)
            .collect();
        for b in &values {
            if !self.bindings.iter().any(|old| old.vk == b.vk) {
                self.down[b.vk] = is_down(b.vk)
            }
        }
        self.bindings = values;
        released
    }
    fn reconcile(&mut self, mut is_down: impl FnMut(usize) -> bool) -> Vec<usize> {
        let mut events = vec![];
        let keys: Vec<_> = self
            .active
            .iter()
            .map(|b| b.vk)
            .chain(MODIFIER_KEYS)
            .collect();
        for vk in keys {
            // 抑止したkeydownはOSの非同期状態へ反映されない。物理keyupで解除する。
            if self.down[vk] && !self.suppressed[vk] && !is_down(vk) {
                events.extend(self.event(vk, false))
            }
        }
        events
    }
}
struct HookState {
    target: HWND,
    message: u32,
    keys: KeyState,
}
thread_local! {static STATE:RefCell<Option<HookState>>=const{RefCell::new(None)};}
fn post(state: &HookState, events: Vec<usize>) {
    for action in events {
        unsafe {
            let _ = PostMessageW(Some(state.target), state.message, WPARAM(action), LPARAM(0));
        }
    }
}
unsafe extern "system" fn keyboard_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && l.0 != 0 {
        let pressed = match w.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
            WM_KEYUP | WM_SYSKEYUP => Some(false),
            _ => None,
        };
        let key = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        if let Some(pressed) = pressed.filter(|_| !key.flags.contains(LLKHF_INJECTED)) {
            let suppress = STATE.with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    if let Some(state) = slot.as_mut() {
                        let (events, suppress) = state.keys.observe(key.vkCode as usize, pressed);
                        post(state, events);
                        return suppress;
                    }
                }
                false
            });
            if suppress {
                return LRESULT(1);
            }
        }
    }
    // 未割当・共存・合成入力・負のhook codeは必ず後続へ渡す。
    unsafe { CallNextHookEx(None, code, w, l) }
}
fn is_mstsc(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        result.is_ok()
            && String::from_utf16_lossy(&path[..len as usize])
                .rsplit(['\\', '/'])
                .next()
                .is_some_and(|n| n.eq_ignore_ascii_case("mstsc.exe"))
    }
}
fn install_hook() -> Result<HHOOK, String> {
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module.into()), 0)
            .map_err(|e| format!("キー検出を開始できません: {e}"))
    }
}
enum Command {
    Bindings(Vec<Binding>, mpsc::SyncSender<()>),
    Stop,
}
/// 名前は従来の内部APIを保つが、通常アプリとMSTSCの両方を観測する。
pub struct RemoteHotkeys {
    commands: Sender<Command>,
    thread_id: u32,
    worker: Option<JoinHandle<()>>,
}
impl RemoteHotkeys {
    pub fn new(hwnd: HWND, message: u32) -> Result<Self, String> {
        let (commands, receive) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let raw = hwnd.0 as usize;
        let worker = thread::Builder::new()
            .name("mic-shortcuts".into())
            .spawn(move || run(raw, message, receive, ready))
            .map_err(|e| e.to_string())?;
        match started.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                commands,
                thread_id,
                worker: Some(worker),
            }),
            status => {
                let _ = worker.join();
                Err(status
                    .ok()
                    .and_then(Result::err)
                    .unwrap_or_else(|| "キー検出が終了しました".into()))
            }
        }
    }
    pub fn set_bindings(&self, values: &[(usize, Hotkey)]) -> Result<(), String> {
        let values = bindings(values)?;
        let (applied, receive) = mpsc::sync_channel(1);
        self.commands
            .send(Command::Bindings(values, applied))
            .map_err(|_| "キー検出が終了しています".to_owned())?;
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WAKE, WPARAM(0), LPARAM(0));
        }
        receive
            .recv()
            .map_err(|_| "キー設定を反映できません".into())
    }
}
impl Drop for RemoteHotkeys {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WAKE, WPARAM(0), LPARAM(0));
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn run(
    raw: usize,
    message: u32,
    commands: Receiver<Command>,
    ready: mpsc::SyncSender<Result<u32, String>>,
) {
    let mut keys = KeyState::new();
    for vk in MODIFIER_KEYS {
        keys.down[vk] = unsafe { GetAsyncKeyState(vk as i32) } < 0;
    }
    STATE.with(|slot| {
        *slot.borrow_mut() = Some(HookState {
            target: HWND(raw as *mut _),
            message,
            keys,
        })
    });
    let Ok(mut hook) = install_hook().inspect_err(|e| {
        let _ = ready.send(Err(e.clone()));
    }) else {
        return;
    };
    let timer = unsafe { SetTimer(None, 0, 50, None) };
    if timer == 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
        let _ = ready.send(Err("キー監視タイマーを開始できません".into()));
        return;
    }
    let mut msg = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
    }
    if ready.send(Ok(unsafe { GetCurrentThreadId() })).is_err() {
        unsafe {
            let _ = KillTimer(None, timer);
            let _ = UnhookWindowsHookEx(hook);
        }
        return;
    }
    let mut last_refresh = Instant::now();
    'messages: loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Bindings(values, applied) => STATE.with(|slot| {
                    if let Some(s) = slot.borrow_mut().as_mut() {
                        let events = s.keys.replace_bindings(
                            values,
                            |vk| unsafe { GetAsyncKeyState(vk as i32) } < 0,
                        );
                        post(s, events);
                    }
                    let _ = applied.send(());
                }),
                Command::Stop => break 'messages,
            }
        }
        let remote = is_mstsc(unsafe { GetForegroundWindow() });
        if remote && last_refresh.elapsed() >= Duration::from_millis(500) {
            if let Ok(next) = install_hook() {
                unsafe {
                    let _ = UnhookWindowsHookEx(hook);
                }
                hook = next;
            }
            last_refresh = Instant::now();
        }
        // 通常前景では取り逃した解放も回収。MSTSC中の非同期キー状態は使わない。
        if !remote {
            STATE.with(|slot| {
                if let Some(s) = slot.borrow_mut().as_mut() {
                    let events = s
                        .keys
                        .reconcile(|vk| unsafe { GetAsyncKeyState(vk as i32) } < 0);
                    post(s, events);
                }
            });
        }
        if unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 <= 0 {
            break;
        }
    }
    STATE.with(|slot| {
        if let Some(s) = slot.borrow_mut().as_mut() {
            let events = s.keys.replace_bindings(vec![], |_| false);
            post(s, events);
        }
    });
    unsafe {
        let _ = KillTimer(None, timer);
        let _ = UnhookWindowsHookEx(hook);
    }
    STATE.with(|slot| *slot.borrow_mut() = None);
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{PTT1, PTT2};
    fn state() -> KeyState {
        let mut s = KeyState::new();
        s.bindings = vec![
            Binding {
                action: 1,
                modifiers: 1,
                vk: 0x74,
                handling: KeyHandling::Coexist,
            },
            Binding {
                action: PTT1,
                modifiers: 1,
                vk: 0x75,
                handling: KeyHandling::Coexist,
            },
            Binding {
                action: PTT2,
                modifiers: 1,
                vk: 0x76,
                handling: KeyHandling::Coexist,
            },
        ];
        s
    }
    #[test]
    fn override_suppresses_matched_down_repeats_and_up_even_after_rebind() {
        let mut s = state();
        s.bindings[0].handling = KeyHandling::Override;
        assert_eq!(s.observe(0xa4, true), (vec![], false));
        assert_eq!(s.observe(0x74, true), (vec![1], true));
        assert_eq!(s.observe(0x74, true), (vec![], true));
        assert_eq!(s.observe(0x76, true), (vec![PTT2], false));
        assert_eq!(s.observe(0x76, false), (vec![PTT2 + 3], false));
        s.replace_bindings(vec![], |_| false);
        assert_eq!(s.observe(0xa4, false), (vec![], false));
        assert_eq!(s.observe(0x74, false), (vec![], true));
        assert_eq!(s.observe(0x74, true), (vec![], false));
    }
    #[test]
    fn override_ptt_survives_os_async_state_and_releases_when_modifier_is_lifted() {
        let mut s = state();
        s.bindings[1].handling = KeyHandling::Override;
        s.observe(0xa4, true);
        assert_eq!(s.observe(0x75, true), (vec![PTT1], true));
        assert!(s.reconcile(|vk| vk == 0xa4).is_empty());
        assert_eq!(s.observe(0xa4, false), (vec![PTT1 + 3], false));
        assert_eq!(s.observe(0x75, false), (vec![], true));
        assert_eq!(s.observe(0x75, true), (vec![], false));
    }
    #[test]
    fn coexist_and_wrong_modifiers_pass_through_every_event() {
        let mut s = state();
        s.bindings[0].handling = KeyHandling::Override;
        assert_eq!(s.observe(0x74, true), (vec![], false));
        s.observe(0x74, false);
        s.observe(0xa4, true);
        s.observe(0xa0, true);
        assert_eq!(s.observe(0x74, true), (vec![], false));
        s.observe(0x74, false);
        s.observe(0xa0, false);
        assert_eq!(s.observe(0x75, true), (vec![PTT1], false));
        assert_eq!(s.observe(0x75, false), (vec![PTT1 + 3], false));
    }
    #[test]
    fn exact_modifiers_and_repeat() {
        let mut s = state();
        assert!(s.event(0x74, true).is_empty());
        s.event(0x74, false);
        s.event(0xa4, true);
        assert_eq!(s.event(0x74, true), vec![1]);
        assert!(s.event(0x74, true).is_empty());
        assert!(s.event(0x74, false).is_empty());
        s.event(0xa0, true);
        assert!(s.event(0x74, true).is_empty());
    }
    #[test]
    fn held_outputs_release_independently_even_if_modifiers_release_first() {
        let mut s = state();
        s.event(0xa5, true);
        assert_eq!(s.event(0x75, true), vec![PTT1]);
        assert_eq!(s.event(0x76, true), vec![PTT2]);
        assert!(s.event(0x75, true).is_empty());
        assert_eq!(s.event(0x75, false), vec![PTT1 + 3]);
        assert_eq!(s.event(0xa5, false), vec![PTT2 + 3]);
        assert!(s.event(0x76, false).is_empty());
    }
    #[test]
    fn rebinding_and_missing_keyup_release_ptt_without_retrigger() {
        let mut s = state();
        s.event(0xa4, true);
        s.event(0x75, true);
        assert_eq!(s.reconcile(|vk| vk != 0x75), vec![PTT1 + 3]);
        s.event(0x75, true);
        let b = s.bindings.clone();
        assert_eq!(s.replace_bindings(b, |_| true), vec![PTT1 + 3]);
        assert!(s.event(0x75, true).is_empty());
    }
    #[test]
    fn disabled_and_duplicate_bindings() {
        let mut key = super::super::config::default_hotkeys()[0].clone();
        assert!(bindings(&[(1, key.clone()), (2, key.clone())]).is_err());
        key.enabled = false;
        assert!(bindings(&[(1, key)]).unwrap().is_empty());
    }
}
