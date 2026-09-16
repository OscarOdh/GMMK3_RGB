//! `config.json` — read once at startup, written only when the user clicks Save.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::{Path, PathBuf};

// LED layout, mapped by lighting each index one at a time on the real board.
// It is not what the original spec described, and GET_DEVICE_INFO agrees with
// the board rather than the spec: it reports 125 LEDs, not 124.
//
//     0..=103  main typing matrix
//   104..=112  left underglow, index 104 at the top
//        113   nothing — no LED wired here
//   114..=122  right underglow, index 114 at the *bottom*
//        123   nothing — no LED wired here
//        124   knob accent

/// Total LEDs the firmware expects. Matches `GET_DEVICE_INFO`.
pub const LED_COUNT: usize = 125;
/// Bytes in one full frame (RGB triplets).
pub const FRAME_BYTES: usize = LED_COUNT * 3;

/// Main typing matrix.
pub const MAIN_START: usize = 0;
pub const MAIN_END: usize = 104;
pub const MAIN_KEY_COUNT: usize = MAIN_END - MAIN_START;
/// Left underglow strip, top to bottom.
pub const LEFT_UG_START: usize = 104;
pub const LEFT_UG_END: usize = 113;
/// Right underglow strip, bottom to top.
pub const RIGHT_UG_START: usize = 114;
pub const RIGHT_UG_END: usize = 123;
/// Knob accent.
pub const KNOB_START: usize = 124;
pub const KNOB_END: usize = 125;
/// Indexes the firmware counts but nothing is wired to. Held at black so a
/// frame is fully determined rather than carrying stale bytes.
pub const UNUSED_LEDS: [usize; 2] = [113, 123];

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// An `#RRGGBB` colour. Stored as raw bytes, serialised as a hex string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HexColor(pub [u8; 3]);

impl HexColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b])
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0[0], self.0[1], self.0[2])
    }

    /// Accepts `#RRGGBB`, `RRGGBB`, `#RGB` and `RGB`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        match s.len() {
            6 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                Some(Self([(v >> 16) as u8, (v >> 8) as u8, v as u8]))
            }
            3 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                let r = ((v >> 8) & 0xF) as u8;
                let g = ((v >> 4) & 0xF) as u8;
                let b = (v & 0xF) as u8;
                Some(Self([r * 0x11, g * 0x11, b * 0x11]))
            }
            _ => None,
        }
    }
}

impl Serialize for HexColor {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid hex colour {s:?}")))
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

fn def_main() -> HexColor {
    HexColor::new(0x00, 0xFF, 0x00)
}
fn def_left() -> HexColor {
    HexColor::new(0x00, 0x19, 0x00)
}
fn def_right() -> HexColor {
    HexColor::new(0xFF, 0xF0, 0xC8)
}
fn def_knob() -> HexColor {
    HexColor::new(0x00, 0x00, 0x00)
}
fn def_press() -> HexColor {
    HexColor::new(0x00, 0x00, 0xFF)
}
fn def_fade() -> f32 {
    0.65
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zones {
    #[serde(default = "def_main")]
    pub main_keys: HexColor,
    #[serde(default = "def_left")]
    pub left_underglow: HexColor,
    #[serde(default = "def_right")]
    pub right_underglow: HexColor,
    #[serde(default = "def_knob")]
    pub knob_accent: HexColor,
}

impl Default for Zones {
    fn default() -> Self {
        Self {
            main_keys: def_main(),
            left_underglow: def_left(),
            right_underglow: def_right(),
            knob_accent: def_knob(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReactiveEffect {
    #[serde(default = "def_press")]
    pub press_color: HexColor,
    #[serde(default = "def_fade")]
    pub fade_duration_sec: f32,
}

impl Default for ReactiveEffect {
    fn default() -> Self {
        Self {
            press_color: def_press(),
            fade_duration_sec: def_fade(),
        }
    }
}

fn def_caps_color() -> HexColor {
    HexColor::new(0x00, 0xFF, 0x00)
}
fn def_true() -> bool {
    true
}

/// Paint the knob accent LED a different colour while Caps Lock is on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapsLockIndicator {
    #[serde(default = "def_true")]
    pub enabled: bool,
    #[serde(default = "def_caps_color")]
    pub color: HexColor,
}

impl Default for CapsLockIndicator {
    fn default() -> Self {
        Self {
            enabled: true,
            color: def_caps_color(),
        }
    }
}

fn def_direct_mode() -> u8 {
    crate::protocol::DEFAULT_DIRECT_MODE
}

/// Transport settings. Everything here has a working default; the fields exist
/// so a firmware quirk can be corrected without a rebuild.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Optional hard filter. `null` means "take the first QMK Raw HID endpoint
    /// found", which is the normal case.
    #[serde(default)]
    pub vid: Option<String>,
    #[serde(default)]
    pub pid: Option<String>,
    /// Raw HID report size. `null` reads it from the device's HID report
    /// descriptor, which is what you want.
    #[serde(default)]
    pub packet_size: Option<usize>,
    /// The firmware effect id that hands every LED to the host. See
    /// [`crate::protocol::DEFAULT_DIRECT_MODE`].
    #[serde(default = "def_direct_mode")]
    pub direct_mode: u8,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            vid: None,
            pid: None,
            packet_size: None,
            direct_mode: def_direct_mode(),
        }
    }
}

impl DeviceConfig {
    pub fn parsed_ids(&self) -> (Option<u16>, Option<u16>) {
        (
            parse_usb_id(self.vid.as_deref()),
            parse_usb_id(self.pid.as_deref()),
        )
    }
}

/// `"0x320F"`, `"320F"` and `"12815"` all work.
fn parse_usb_id(s: Option<&str>) -> Option<u16> {
    let s = s?.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u16::from_str_radix(hex, 16).ok();
    }
    s.parse::<u16>()
        .ok()
        .or_else(|| u16::from_str_radix(s, 16).ok())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub zones: Zones,
    #[serde(default)]
    pub reactive_effect: ReactiveEffect,
    #[serde(default)]
    pub caps_lock_indicator: CapsLockIndicator,
    /// Launch straight to the tray with no window.
    #[serde(default)]
    pub start_hidden: bool,
    #[serde(default)]
    pub device: DeviceConfig,
}

impl Config {
    /// Returns the config plus a human-readable warning if the file existed but
    /// could not be used. A missing file is not an error — defaults are used.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(cfg) => (cfg, None),
                Err(e) => (
                    Self::default(),
                    Some(format!("config.json is not valid ({e}) — using defaults")),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (
                Self::default(),
                Some(format!("could not read config.json ({e}) — using defaults")),
            ),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        text.push('\n');
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Both files live next to the executable so the tool stays portable.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub keymap: PathBuf,
}

impl Paths {
    pub fn discover() -> Self {
        let dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            config: dir.join("config.json"),
            keymap: dir.join("keymap.json"),
        }
    }
}
