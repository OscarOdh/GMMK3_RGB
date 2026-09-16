//! QMK OpenRGB protocol over Raw HID.
//!
//! The endpoint is found by HID usage rather than VID/PID so it keeps working
//! across firmware rebuilds. The report size is read out of the device's own
//! HID report descriptor rather than assumed, because getting it wrong means
//! writing malformed reports at 60 FPS — which is a good way to wedge the
//! keyboard's USB endpoint and lose key-up events.

use crate::config::DeviceConfig;
use hidapi::{HidApi, HidDevice};

/// QMK's Raw HID interface advertises this usage page / usage pair.
pub const RAW_USAGE_PAGE: u16 = 0xFF60;
pub const RAW_USAGE: u16 = 0x61;

/// QMK's stock `RAW_EPSIZE`, used when the descriptor cannot be read.
pub const DEFAULT_PACKET_SIZE: usize = 32;
/// Biggest report we size buffers for.
pub const MAX_PACKET_SIZE: usize = 64;
/// Smallest useful payload: a command plus one RGB triplet. Used only to
/// sanity-check a detected or configured report size.
pub const MIN_PAYLOAD_BYTES: usize = 4;

pub const CMD_GET_PROTOCOL_VERSION: u8 = 0x01;
pub const CMD_GET_QMK_VERSION: u8 = 0x02;
pub const CMD_GET_DEVICE_INFO: u8 = 0x03;
pub const CMD_GET_MODE_INFO: u8 = 0x04;
pub const CMD_GET_LED_INFO: u8 = 0x05;
pub const CMD_GET_ENABLED_MODES: u8 = 0x06;
pub const CMD_SET_MODE: u8 = 0x07;
pub const CMD_DIRECT_MODE_SET_SINGLE_LED: u8 = 0x08;

/// Every read-only command, for the diagnostics report.
pub const GET_COMMANDS: [(u8, &str); 6] = [
    (CMD_GET_PROTOCOL_VERSION, "GET_PROTOCOL_VERSION"),
    (CMD_GET_QMK_VERSION, "GET_QMK_VERSION"),
    (CMD_GET_DEVICE_INFO, "GET_DEVICE_INFO"),
    (CMD_GET_MODE_INFO, "GET_MODE_INFO"),
    (CMD_GET_LED_INFO, "GET_LED_INFO"),
    (CMD_GET_ENABLED_MODES, "GET_ENABLED_MODES"),
];

/// QMK appends the OpenRGB direct effect *after* the built-in RGB matrix
/// effects. This firmware enables 44 of them (`Static` = 1 … `Pixel Fractal` =
/// 44), so direct lands at 45 — confirmed against the OpenRGB profile dumped
/// from this exact keyboard.
///
/// It is emphatically **not** 0: that is `RGB_MATRIX_NONE`, which switches the
/// whole board off.
pub const DEFAULT_DIRECT_MODE: u8 = 45;

pub struct Keyboard {
    dev: HidDevice,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub packet_size: usize,
    /// Where `packet_size` came from, for the status line.
    pub packet_size_source: &'static str,
    /// Reply to `GET_PROTOCOL_VERSION`, shown in the GUI so a protocol mismatch
    /// is visible instead of silent.
    pub handshake: String,
}

/// One candidate Raw HID endpoint.
pub struct Candidate {
    pub path: std::ffi::CString,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub interface: i32,
}

/// Every QMK Raw HID endpoint on the system, in enumeration order.
pub fn candidates(api: &HidApi) -> Vec<Candidate> {
    api.device_list()
        .filter(|i| i.usage_page() == RAW_USAGE_PAGE && i.usage() == RAW_USAGE)
        .map(|i| Candidate {
            path: i.path().to_owned(),
            name: i.product_string().unwrap_or("QMK device").to_string(),
            vid: i.vendor_id(),
            pid: i.product_id(),
            interface: i.interface_number(),
        })
        .collect()
}

/// Find and open the endpoint, honouring the optional vid/pid filter.
pub fn open(api: &mut HidApi, cfg: &DeviceConfig) -> Result<Keyboard, String> {
    // Windows lets several processes hold the same HID device at once, so a
    // second RGB program does not fail loudly — it just writes over us, and we
    // over it. Refusing is the only way to make the lighting predictable.
    let rivals = crate::conflicts::running();
    if !rivals.is_empty() {
        return Err(format!(
            "{} is running and drives the same Raw HID endpoint. Close it — two programs \
             streaming to this keyboard fight each other and neither wins.",
            rivals.join(", ")
        ));
    }

    api.refresh_devices()
        .map_err(|e| format!("HID enumeration failed: {e}"))?;

    let (want_vid, want_pid) = cfg.parsed_ids();
    let all = candidates(api);
    let found = all
        .iter()
        .find(|c| !want_vid.is_some_and(|v| c.vid != v) && !want_pid.is_some_and(|p| c.pid != p))
        .ok_or_else(|| {
            if all.is_empty() {
                "no QMK Raw HID endpoint (usage page 0xFF60, usage 0x61) found — is the keyboard plugged in?"
                    .to_string()
            } else {
                "a QMK Raw HID endpoint exists but none matched the vid/pid filter in config.json".to_string()
            }
        })?;

    let dev = api.open_path(&found.path).map_err(|e| {
        format!(
            "found {} but could not open it ({e}) — another RGB app may hold the endpoint",
            found.name
        )
    })?;

    // Never guess this. Writing wrong-sized reports stalls the endpoint, and a
    // stalled endpoint costs you key-up events — the keyboard starts repeating
    // whatever you last pressed until it is replugged. Refusing to run is far
    // better than falling back to a plausible-looking default.
    let (packet_size, packet_size_source) = match cfg.packet_size {
        Some(n) if (MIN_PAYLOAD_BYTES..=MAX_PACKET_SIZE).contains(&n) => (n, "from config.json"),
        Some(n) => {
            return Err(format!(
                "config.json sets device.packet_size to {n}, which is outside the usable range \
                 {}..={MAX_PACKET_SIZE}. Set it to null to detect it from the keyboard.",
                MIN_PAYLOAD_BYTES
            ));
        }
        None => match detect_packet_size(&dev) {
            Some(n) => (n, "detected"),
            None => {
                return Err(
                    "could not read the Raw HID report size from the keyboard's HID descriptor. \
                     Run `gmmk3_rgb.exe --probe` and set device.packet_size in config.json to the \
                     value it reports (QMK's RAW_EPSIZE, usually 32 or 64)."
                        .to_string(),
                );
            }
        },
    };

    // Refuse to drive a keyboard that is listening to VIA. Returning an error
    // here means we never send 0x07 or 0x09, which VIA would read as
    // "set custom value" and "save to EEPROM".
    let handshake = handshake(&dev, packet_size);
    if let Handshake::Via { version, .. } = handshake {
        return Err(format!(
            "{} is in VIA mode (VIA protocol v{version}), not OpenRGB mode. \
             Press Fn+O on the keyboard to switch it back, then press Reconnect. \
             Nothing was sent — driving lighting in VIA mode would write to the keyboard's EEPROM.",
            found.name
        ));
    }

    Ok(Keyboard {
        dev,
        name: found.name.clone(),
        vid: found.vid,
        pid: found.pid,
        packet_size,
        packet_size_source,
        handshake: handshake.describe(),
    })
}

/// Read the output report size straight out of the HID report descriptor.
pub fn detect_packet_size(dev: &HidDevice) -> Option<usize> {
    let mut buf = [0u8; 4096];
    let len = dev.get_report_descriptor(&mut buf).ok()?;
    let size = output_report_bytes(&buf[..len])?;
    (MIN_PAYLOAD_BYTES..=MAX_PACKET_SIZE)
        .contains(&size)
        .then_some(size)
}

/// Minimal HID report-descriptor walk: track the global Report Size / Report
/// Count and take the largest Output main item they describe.
fn output_report_bytes(desc: &[u8]) -> Option<usize> {
    let (mut report_size, mut report_count, mut best) = (0u32, 0u32, 0usize);
    let mut i = 0;
    while i < desc.len() {
        let head = desc[i];
        if head == 0xFE {
            // Long item: [0xFE][data len][tag][data…]
            let data_len = *desc.get(i + 1)? as usize;
            i += 3 + data_len;
            continue;
        }
        let len = match head & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let end = i + 1 + len;
        if end > desc.len() {
            break;
        }
        // Item data is little-endian.
        let value = desc[i + 1..end]
            .iter()
            .rev()
            .fold(0u32, |acc, &b| (acc << 8) | b as u32);
        match head & 0xFC {
            0x74 => report_size = value,  // Global: Report Size (bits)
            0x94 => report_count = value, // Global: Report Count
            0x90 => {
                // Main: Output
                let bytes = report_size.saturating_mul(report_count) / 8;
                best = best.max(bytes as usize);
            }
            _ => {}
        }
        i = end;
    }
    (best > 0).then_some(best)
}

/// What answered `GET_PROTOCOL_VERSION`.
///
/// This matters far more than it looks. On a `viahybrid` keymap the same Raw
/// HID endpoint serves both protocols, and the command bytes collide badly:
/// OpenRGB's `SET_MODE` (0x07) is VIA's `id_custom_set_value`, and OpenRGB's
/// `DIRECT_MODE_SET_LEDS` (0x09) is VIA's `id_custom_save` — an EEPROM write.
/// Streaming frames at 60 FPS into a keyboard in VIA mode would hammer its
/// EEPROM, stall the firmware's main loop, and cost you key-up events. So we
/// establish which protocol is listening *before* sending anything else.
pub enum Handshake {
    OpenRgb {
        version: u8,
        raw: String,
    },
    /// VIA answers with a big-endian u16, so its high byte is zero.
    Via {
        version: u16,
        raw: String,
    },
    NoReply,
    Unknown {
        raw: String,
    },
}

impl Handshake {
    pub fn describe(&self) -> String {
        match self {
            Self::OpenRgb { version, raw } => format!("v{version} (raw {raw})"),
            Self::Via { version, raw } => format!("VIA v{version} (raw {raw})"),
            Self::NoReply => "no reply".to_string(),
            Self::Unknown { raw } => format!("unrecognised (raw {raw})"),
        }
    }
}

/// Sends only `GET_PROTOCOL_VERSION` (0x01), which is a pure read in *both*
/// protocols — VIA numbers its own `id_get_protocol_version` 0x01 too. Safe to
/// call whatever mode the keyboard is in.
pub fn handshake(dev: &HidDevice, packet_size: usize) -> Handshake {
    let mut out = [0u8; MAX_PACKET_SIZE + 1];
    out[1] = CMD_GET_PROTOCOL_VERSION;
    if dev.write(&out[..=packet_size]).is_err() {
        return Handshake::NoReply;
    }
    let mut reply = [0u8; MAX_PACKET_SIZE];
    let Ok(n) = dev.read_timeout(&mut reply[..packet_size], 250) else {
        return Handshake::NoReply;
    };
    if n < 3 {
        return if n == 0 {
            Handshake::NoReply
        } else {
            Handshake::Unknown {
                raw: hex(&reply[..n]),
            }
        };
    }
    let raw = hex(&reply[..n.min(6)]);
    // Both protocols echo the command in byte 0 and answer from byte 1.
    // OpenRGB replies with a single-byte version; VIA with a big-endian u16,
    // whose high byte is always 0 for any version that exists.
    match (reply[0], reply[1], reply[2]) {
        (CMD_GET_PROTOCOL_VERSION, 0, lo) if lo != 0 => Handshake::Via {
            version: lo as u16,
            raw,
        },
        (CMD_GET_PROTOCOL_VERSION, version, _) if version != 0 => {
            Handshake::OpenRgb { version, raw }
        }
        _ => Handshake::Unknown { raw },
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl Keyboard {
    /// Send a read-only `GET_*` command and hand back the raw reply.
    pub fn query(&self, command: u8) -> Result<Vec<u8>, String> {
        let mut out = [0u8; MAX_PACKET_SIZE + 1];
        out[1] = command;
        self.dev
            .write(&out[..=self.packet_size])
            .map_err(|e| e.to_string())?;
        let mut reply = [0u8; MAX_PACKET_SIZE];
        match self.dev.read_timeout(&mut reply[..self.packet_size], 300) {
            Ok(0) => Err("no reply".to_string()),
            Ok(n) => Ok(reply[..n].to_vec()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// `DIRECT_MODE_SET_SINGLE_LED` — `[cmd][led][r][g][b]`.
    ///
    /// The only write in this protocol confirmed against real hardware: every
    /// LED index in `config.rs` was mapped by calling this one index at a time
    /// and looking at the keyboard. It is also incapable of overrunning the
    /// packet, unlike the bulk `SET_LEDS` (0x09) this codebase deliberately
    /// does not use — see the README.
    pub fn set_single_led(&self, led: usize, rgb: [u8; 3]) -> Result<(), String> {
        let mut out = [0u8; MAX_PACKET_SIZE + 1];
        out[1] = CMD_DIRECT_MODE_SET_SINGLE_LED;
        out[2] = led as u8;
        out[3] = rgb[0];
        out[4] = rgb[1];
        out[5] = rgb[2];
        self.dev
            .write(&out[..=self.packet_size])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Consume any input reports the firmware sent back, so replies cannot pile
    /// up behind us. Non-blocking; a device with nothing to say costs nothing.
    pub fn drain_input(&self) {
        let mut scratch = [0u8; MAX_PACKET_SIZE];
        for _ in 0..8 {
            match self.dev.read_timeout(&mut scratch[..self.packet_size], 0) {
                Ok(n) if n > 0 => continue,
                _ => break,
            }
        }
    }

    /// The effect id the firmware is currently running, per `GET_MODE_INFO`.
    pub fn current_mode(&self) -> Option<u8> {
        let reply = self.query(CMD_GET_MODE_INFO).ok()?;
        (reply.len() >= 2 && reply[0] == CMD_GET_MODE_INFO).then(|| reply[1])
    }

    /// Switch the keyboard into `mode`, and confirm it took.
    ///
    /// This is what selecting "Direct" in OpenRGB does. The payload layout
    /// comes from the QMK OpenRGB firmware itself:
    ///
    /// ```c
    /// void openrgb_set_mode(uint8_t *data) {
    ///     const uint8_t h     = data[1];
    ///     const uint8_t s     = data[2];
    ///     const uint8_t v     = data[3];
    ///     const uint8_t mode  = data[4];
    ///     const uint8_t speed = data[5];
    ///     const uint8_t save  = data[6];
    /// }
    /// ```
    ///
    /// The mode sits at index **4**, behind hue, saturation and value — not
    /// immediately after the command byte, where every reasonable guess put it.
    /// Writing it at index 1 lands it in the hue byte, which is why earlier
    /// builds kept changing the board's *colour* while the effect never moved:
    /// a mode id of 45 became a hue of 45, and the accompanying padding became
    /// a saturation low enough to wash the board white.
    ///
    /// `save` is left 0, so the mode is set without a write to EEPROM.
    pub fn set_direct_mode(&self, mode: u8) -> Result<String, String> {
        let mut out = [0u8; MAX_PACKET_SIZE + 1];
        out[1] = CMD_SET_MODE;
        out[2] = 0; // hue
        out[3] = 255; // saturation
        out[4] = 255; // value — brightness; a zero here blanks the board
        out[5] = mode;
        out[6] = 127; // speed
        out[7] = 0; // save: do not touch EEPROM
        self.dev
            .write(&out[..=self.packet_size])
            .map_err(|e| e.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(120));
        self.drain_input();

        // Verified, because a SET_MODE that quietly does nothing is precisely
        // the failure this spent several rounds hiding behind.
        match self.current_mode() {
            Some(m) if m == mode => Ok(format!("mode {mode}")),
            Some(m) => Err(format!(
                "asked for mode {mode} but the keyboard reports mode {m}"
            )),
            None => Ok(format!("mode {mode} (could not read back)")),
        }
    }
}
