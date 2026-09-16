//! Detects other RGB software that fights us for the Raw HID endpoint.
//!
//! This matters more than it sounds. hidapi opens HID devices shareable on
//! Windows, so a second program can hold the same endpoint and write to it at
//! the same time — without any error from either side. The symptoms are
//! indistinguishable from a protocol bug: writes succeed, LEDs do whatever the
//! *other* program says, and contention on the endpoint can stall transfers.
//!
//! It also ruins diagnostics. "The LED turned red" only means something if
//! nothing else could have turned it red.

use std::mem::size_of;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

/// Executable names (lowercase, without `.exe`) known to drive keyboard RGB.
const KNOWN: &[(&str, &str)] = &[
    ("openrgb", "OpenRGB"),
    ("gloriouscore", "Glorious Core"),
    ("glorious core", "Glorious Core"),
    ("artemis.ui", "Artemis"),
    ("artemis", "Artemis"),
    ("signalrgb", "SignalRGB"),
    ("via", "VIA"),
    ("qmk toolbox", "QMK Toolbox"),
    ("qmk_toolbox", "QMK Toolbox"),
];

/// Friendly names of conflicting programs currently running.
pub fn running() -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for name in process_names() {
        let lower = name.to_ascii_lowercase();
        let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
        for (needle, label) in KNOWN {
            if stem == *needle || stem.starts_with(needle) {
                if !found.iter().any(|f| f == label) {
                    found.push((*label).to_string());
                }
                break;
            }
        }
    }
    found
}

fn process_names() -> Vec<String> {
    let mut names = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return names;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                names.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    names
}
