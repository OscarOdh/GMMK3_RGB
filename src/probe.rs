//! `gmmk3_rgb.exe --probe` — a read-only diagnostic with no window.
//!
//! `--debug` is the tool to reach for; this exists for the case that one cannot
//! cover, which is a machine where the GUI itself will not start. It writes
//! `probe.txt` next to the exe and shows a message box saying where.
//!
//! It deliberately **never writes to the device**, so it is safe to run against
//! a keyboard that is already misbehaving.

use crate::config::Paths;
use crate::protocol;
use hidapi::HidApi;
use std::fmt::Write as _;
use std::ptr;
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};

pub fn run() {
    let mut report = String::new();
    let _ = writeln!(
        report,
        "GMMK3 RGB — device probe (read-only, no writes sent)\n"
    );

    match HidApi::new() {
        Ok(api) => {
            let found = protocol::candidates(&api);
            let _ = writeln!(
                report,
                "QMK Raw HID endpoints (usage page {:#06X}, usage {:#04X}): {}\n",
                protocol::RAW_USAGE_PAGE,
                protocol::RAW_USAGE,
                found.len()
            );

            for (n, c) in found.iter().enumerate() {
                let _ = writeln!(report, "--- endpoint {n} ---");
                let _ = writeln!(report, "  product   : {}", c.name);
                let _ = writeln!(report, "  vid:pid   : {:04X}:{:04X}", c.vid, c.pid);
                let _ = writeln!(report, "  interface : {}", c.interface);
                let _ = writeln!(report, "  path      : {}", c.path.to_string_lossy());

                match api.open_path(&c.path) {
                    Ok(dev) => {
                        let packet_size = protocol::detect_packet_size(&dev)
                            .unwrap_or(protocol::DEFAULT_PACKET_SIZE);
                        // Pure read in both OpenRGB and VIA mode — see
                        // protocol::handshake.
                        let shake = protocol::handshake(&dev, packet_size);
                        let _ = writeln!(report, "  protocol  : {}", shake.describe());
                        if let protocol::Handshake::Via { .. } = shake {
                            let _ = writeln!(
                                report,
                                "  >>> The keyboard is in VIA mode. Press Fn+O to switch it to \
                                 OpenRGB mode before running the app."
                            );
                        }
                        match protocol::detect_packet_size(&dev) {
                            Some(size) => {
                                let _ = writeln!(report, "  packet    : {size} bytes");
                            }
                            None => {
                                let _ = writeln!(
                                    report,
                                    "  packet    : could not be read from the descriptor — the \
                                     app will refuse to run rather than guess"
                                );
                            }
                        }
                        let mut buf = [0u8; 4096];
                        match dev.get_report_descriptor(&mut buf) {
                            Ok(len) => {
                                let _ = writeln!(report, "  descriptor ({len} bytes):");
                                for line in buf[..len].chunks(16) {
                                    let hex: Vec<String> =
                                        line.iter().map(|b| format!("{b:02X}")).collect();
                                    let _ = writeln!(report, "    {}", hex.join(" "));
                                }
                            }
                            Err(e) => {
                                let _ = writeln!(report, "  descriptor: unavailable ({e})");
                            }
                        }
                    }
                    Err(e) => {
                        let _ = writeln!(
                            report,
                            "  could not open ({e}) — another RGB app may hold the endpoint"
                        );
                    }
                }
                let _ = writeln!(report);
            }

            let _ = writeln!(report, "--- all HID devices ---");
            for info in api.device_list() {
                let _ = writeln!(
                    report,
                    "  {:04X}:{:04X}  usage {:#06X}/{:#04X}  if {}  {}",
                    info.vendor_id(),
                    info.product_id(),
                    info.usage_page(),
                    info.usage(),
                    info.interface_number(),
                    info.product_string().unwrap_or("")
                );
            }
        }
        Err(e) => {
            let _ = writeln!(report, "could not initialise hidapi: {e}");
        }
    }

    let path = Paths::discover().config.with_file_name("probe.txt");
    let message = match std::fs::write(&path, &report) {
        Ok(()) => format!("Probe written to:\n{}", path.display()),
        Err(e) => format!("Could not write {}:\n{e}\n\n{report}", path.display()),
    };
    message_box(&message);
}

fn message_box(text: &str) {
    let body = crate::input::wide(text);
    let title = crate::input::wide("GMMK3 RGB — device probe");
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}
