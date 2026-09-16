//! Win32 side of the app: the low-level keyboard hook that drives the reactive
//! effect, and a hidden top-level window that receives `WM_POWERBROADCAST`.
//!
//! Both live on one dedicated thread because each needs a message pump. The
//! tray gets its own thread — showing its menu blocks the pump, and blocking
//! this one would stall keyboard input system-wide.
//!
//! The hook talks to the render loop through plain atomics, so the callback
//! never allocates, never locks, and always returns immediately.

use crate::keymap::{KEY_SLOTS, key_id};
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HHOOK,
    KBDLLHOOKSTRUCT, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    RegisterClassW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_POWERBROADCAST, WM_SYSKEYDOWN, WNDCLASSW, WS_OVERLAPPED,
};

/// `HC_ACTION` — the hook may act on this event.
const HC_ACTION: i32 = 0;
/// `LLKHF_EXTENDED` — the key came from an E0-prefixed scan code.
const LLKHF_EXTENDED: u32 = 0x01;
/// `PBT_APMRESUMECRITICAL`, sent when the machine woke from an unexpected halt.
const PBT_APMRESUMECRITICAL: u32 = 6;

/// How long to let the USB bus re-enumerate after a resume before reopening.
pub const RESUME_GRACE_MS: u64 = 3_000;

/// Also used by the single-instance guard to find the running copy.
pub const POWER_WINDOW_CLASS: &str = "GMMK3RgbPowerListener";

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Millisecond timestamp of the last press of each key id. `0` means "never".
pub static PRESS_MS: [AtomicU64; KEY_SLOTS] = [const { AtomicU64::new(0) }; KEY_SLOTS];
/// Timestamp of the most recent press of *any* key. Lets the render loop decide
/// in one atomic load whether anything is still decaying, instead of scanning
/// all 104 mapped slots on every idle frame.
pub static LAST_PRESS_MS: AtomicU64 = AtomicU64::new(0);
/// Most recent key id, plus a counter so the GUI can tell a fresh press from a
/// stale reading while learning the keymap.
pub static LAST_KEY_ID: AtomicU32 = AtomicU32::new(0);
pub static LAST_KEY_SEQ: AtomicU64 = AtomicU64::new(0);

/// Whether Caps Lock is currently on. Seeded once by `seed_caps_lock`, then
/// kept current by the hook.
pub static CAPS_ON: AtomicBool = AtomicBool::new(false);
/// Whether the Caps Lock key is physically held, so typematic repeats of the
/// key-down do not re-toggle `CAPS_ON`.
static CAPS_HELD: AtomicBool = AtomicBool::new(false);

/// Set while the machine is suspended; the render loop drops the USB handle.
pub static SUSPENDED: AtomicBool = AtomicBool::new(false);
/// After a resume, do not touch the bus until this timestamp.
pub static RESUME_GRACE_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

static HOOK: AtomicIsize = AtomicIsize::new(0);
static CLOCK: OnceLock<Instant> = OnceLock::new();

/// Start the monotonic clock. Call once, before any thread is spawned.
pub fn init_clock() {
    let _ = CLOCK.set(Instant::now());
}

/// Milliseconds since startup, biased by 1 so a real timestamp is never `0`
/// (which `PRESS_MS` uses to mean "never pressed").
pub fn now_ms() -> u64 {
    CLOCK
        .get()
        .map_or(1, |t| t.elapsed().as_millis() as u64 + 1)
}

// ---------------------------------------------------------------------------
// Hook + power window
// ---------------------------------------------------------------------------

unsafe extern "system" fn ll_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION {
        let msg = wparam as u32;
        // SAFETY: for HC_ACTION, lparam is a valid KBDLLHOOKSTRUCT.
        let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;

        // Caps Lock is tracked here rather than read back from Windows,
        // because `GetKeyState` answers from a per-thread snapshot that only
        // refreshes as that thread pulls key messages — and none of our threads
        // ever do. The OS toggles the lock on the key-down *transition*, so a
        // held key (typematic repeat) must not toggle again: `CAPS_HELD` masks
        // the repeats until the matching key-up.
        if kb.vkCode == u32::from(VK_CAPITAL) {
            if down {
                if !CAPS_HELD.swap(true, Ordering::Relaxed) {
                    CAPS_ON.fetch_xor(true, Ordering::Relaxed);
                }
            } else {
                CAPS_HELD.store(false, Ordering::Relaxed);
            }
        }

        if down {
            let id = key_id(kb.vkCode, kb.scanCode, kb.flags & LLKHF_EXTENDED != 0) as usize;
            if id < KEY_SLOTS {
                let at = now_ms();
                PRESS_MS[id].store(at, Ordering::Relaxed);
                LAST_PRESS_MS.store(at, Ordering::Relaxed);
                LAST_KEY_ID.store(id as u32, Ordering::Relaxed);
                LAST_KEY_SEQ.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// Capture the Caps Lock state the machine is already in.
///
/// Call this once, early, from a thread that has not yet made any USER32 call:
/// the first `GetKeyState` a thread makes creates its message queue and
/// snapshots the *system* key state into it, toggle bits included. That one
/// read is accurate. Every later read on the same thread would only be as fresh
/// as the last key message it retrieved, which is why the hook takes over from
/// here.
pub fn seed_caps_lock() {
    let on = unsafe { GetKeyState(VK_CAPITAL as i32) } & 0x0001 != 0;
    CAPS_ON.store(on, Ordering::Relaxed);
}

unsafe extern "system" fn power_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // A second launch of the exe posts this instead of starting a rival copy.
    if msg == crate::instance::WM_SHOW_EXISTING {
        crate::tray::request_show();
        return 0;
    }
    if msg == WM_POWERBROADCAST {
        match wparam as u32 {
            PBT_APMSUSPEND => {
                SUSPENDED.store(true, Ordering::SeqCst);
            }
            PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL => {
                RESUME_GRACE_UNTIL_MS.store(now_ms() + RESUME_GRACE_MS, Ordering::SeqCst);
                SUSPENDED.store(false, Ordering::SeqCst);
            }
            _ => {}
        }
        return 1; // TRUE
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// NUL-terminated UTF-16, as every `…W` entry point wants.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn spawn() {
    let _ = std::thread::Builder::new()
        .name("input".to_string())
        .spawn(|| unsafe { run() });
}

unsafe fn run() {
    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };

    // A hidden *top-level* window. Message-only windows (HWND_MESSAGE) are
    // excluded from broadcasts, so WM_POWERBROADCAST would never arrive.
    let class_name = wide(POWER_WINDOW_CLASS);
    let window_name = wide("GMMK3 RGB Power Listener");
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(power_wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: ptr::null_mut(),
        hCursor: ptr::null_mut(),
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    unsafe { RegisterClassW(&wc) };
    // Deliberately never shown, so it stays off the taskbar and Alt+Tab.
    let _hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_name.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            hinstance,
            ptr::null(),
        )
    };

    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_keyboard_proc), hinstance, 0) };
    HOOK.store(hook as isize, Ordering::SeqCst);

    let mut msg = MSG::default();
    loop {
        let got = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
        if got <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Remove the hook on the way out. Windows would clean up anyway, but doing it
/// explicitly avoids a brief input stall while the OS notices the dead process.
pub fn uninstall_hook() {
    let hook = HOOK.swap(0, Ordering::SeqCst);
    if hook != 0 {
        unsafe { UnhookWindowsHookEx(hook as HHOOK) };
    }
}
