//! Single-instance guard.
//!
//! Two copies would fight over the Raw HID handle — the QMK OpenRGB protocol
//! takes it exclusively — and the loser would just sit there showing a
//! "device busy" error. So a second launch hands its intent to the first one
//! (pop the window) and exits quietly.

use crate::input::POWER_WINDOW_CLASS;
use std::ptr;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

/// Session-local (no `Global\` prefix), so two different users signed into the
/// same machine can each run their own copy.
const MUTEX_NAME: &str = "GMMK3_RGB_Controller_SingleInstance";

/// Posted to the first instance's hidden window to ask it to show its GUI.
pub const WM_SHOW_EXISTING: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;

/// Holds the mutex for the life of the process.
pub struct Guard(HANDLE);

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// `None` means another instance already owns the mutex.
pub fn acquire() -> Option<Guard> {
    let name = crate::input::wide(MUTEX_NAME);
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        // Can't tell either way — better to run than to refuse to start.
        return Some(Guard(ptr::null_mut()));
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(handle) };
        return None;
    }
    Some(Guard(handle))
}

/// Ask the running instance to bring its window up. Best effort: it may still
/// be starting, so retry briefly before giving up.
pub fn signal_existing() {
    let class = crate::input::wide(POWER_WINDOW_CLASS);
    for _ in 0..20 {
        let hwnd = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
        if !hwnd.is_null() {
            unsafe { PostMessageW(hwnd, WM_SHOW_EXISTING, 0, 0) };
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
