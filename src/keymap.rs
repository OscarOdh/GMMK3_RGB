//! Maps physical keys to LED indexes 0..104.
//!
//! A key is identified by its PS/2 set-1 scan code plus the extended-key flag,
//! which — unlike a virtual key code — is stable regardless of Num Lock, the
//! active keyboard layout, or which Shift/Ctrl/Alt was pressed.
//!
//! Two exceptions need a synthetic id:
//!
//! * Pause/Break reports scan code 0x45, the same as Num Lock.
//! * Injected events sometimes arrive with scan code 0.
//!
//! Both fall back to `0x200 | vk`, a range no real scan code can reach.

use crate::config::MAIN_KEY_COUNT;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub type KeyId = u16;

/// Size of the press-timestamp table: 0x000..0x1FF for scan codes, 0x200..0x2FF
/// for the virtual-key fallbacks.
pub const KEY_SLOTS: usize = 0x300;

const VK_PAUSE: u32 = 0x13;

/// Collapse a low-level hook event into a stable key id.
pub fn key_id(vk: u32, scan_code: u32, extended: bool) -> KeyId {
    let sc = scan_code & 0xFF;
    if sc == 0 || vk == VK_PAUSE {
        return (0x200 | (vk & 0xFF)) as KeyId;
    }
    (sc | if extended { 0x100 } else { 0 }) as KeyId
}

/// LED order for the GMMK 3 100% ANSI: row by row, left to right, top to
/// bottom, with the nav cluster and numpad included in each row. Verified
/// against the OpenRGB profile dumped from the keyboard, whose LED names run
/// `Key: Escape` (0) … `Key: Number Pad .` (103) in exactly this order.
///
/// If your firmware ever differs, use "Learn Keymap…" in the GUI; it writes
/// `keymap.json` and no rebuild is needed.
///
/// Key id `0` means "this LED has no key that reaches Windows".
#[rustfmt::skip]
pub const DEFAULT_LED_TO_KEY: [(KeyId, &str); MAIN_KEY_COUNT] = [
    // Row 1 — LED 0..15
    (0x001, "Esc"),      (0x03B, "F1"),      (0x03C, "F2"),      (0x03D, "F3"),
    (0x03E, "F4"),       (0x03F, "F5"),      (0x040, "F6"),      (0x041, "F7"),
    (0x042, "F8"),       (0x043, "F9"),      (0x044, "F10"),     (0x057, "F11"),
    (0x058, "F12"),      (0x137, "PrtSc"),   (0x046, "ScrLk"),   (0x213, "Pause"),

    // Row 2 — LED 16..36
    (0x029, "`"),        (0x002, "1"),       (0x003, "2"),       (0x004, "3"),
    (0x005, "4"),        (0x006, "5"),       (0x007, "6"),       (0x008, "7"),
    (0x009, "8"),        (0x00A, "9"),       (0x00B, "0"),       (0x00C, "-"),
    (0x00D, "="),        (0x00E, "Backspace"),
    (0x152, "Insert"),   (0x147, "Home"),    (0x149, "PgUp"),
    (0x045, "NumLock"),  (0x135, "Num /"),   (0x037, "Num *"),   (0x04A, "Num -"),

    // Row 3 — LED 37..57
    (0x00F, "Tab"),      (0x010, "Q"),       (0x011, "W"),       (0x012, "E"),
    (0x013, "R"),        (0x014, "T"),       (0x015, "Y"),       (0x016, "U"),
    (0x017, "I"),        (0x018, "O"),       (0x019, "P"),       (0x01A, "["),
    (0x01B, "]"),        (0x02B, "\\"),
    (0x153, "Delete"),   (0x14F, "End"),     (0x151, "PgDn"),
    (0x047, "Num 7"),    (0x048, "Num 8"),   (0x049, "Num 9"),   (0x04E, "Num +"),

    // Row 4 — LED 58..73
    (0x03A, "CapsLock"), (0x01E, "A"),       (0x01F, "S"),       (0x020, "D"),
    (0x021, "F"),        (0x022, "G"),       (0x023, "H"),       (0x024, "J"),
    (0x025, "K"),        (0x026, "L"),       (0x027, ";"),       (0x028, "'"),
    (0x01C, "Enter"),
    (0x04B, "Num 4"),    (0x04C, "Num 5"),   (0x04D, "Num 6"),

    // Row 5 — LED 74..90
    (0x02A, "LShift"),   (0x02C, "Z"),       (0x02D, "X"),       (0x02E, "C"),
    (0x02F, "V"),        (0x030, "B"),       (0x031, "N"),       (0x032, "M"),
    (0x033, ","),        (0x034, "."),       (0x035, "/"),       (0x036, "RShift"),
    (0x148, "Up"),
    (0x04F, "Num 1"),    (0x050, "Num 2"),   (0x051, "Num 3"),   (0x11C, "Num Enter"),

    // Row 6 — LED 91..103
    (0x01D, "LCtrl"),    (0x15B, "LWin"),    (0x038, "LAlt"),    (0x039, "Space"),
    // LED 96 is Fn, not Right Windows: QMK handles it as a layer key, so it
    // never produces a scan code. (OpenRGB mislabels this position "Key: 4".)
    (0x138, "RAlt"),     (0x000, "Fn"),      (0x15D, "Menu"),    (0x11D, "RCtrl"),
    (0x14B, "Left"),     (0x150, "Down"),    (0x14D, "Right"),
    (0x052, "Num 0"),    (0x053, "Num ."),
];

/// Friendly name for an LED position, used by the Learn Keymap panel.
pub fn default_led_name(led: usize) -> &'static str {
    DEFAULT_LED_TO_KEY.get(led).map(|(_, n)| *n).unwrap_or("?")
}

/// Friendly name for a captured key id, falling back to its raw code.
pub fn key_name(id: KeyId) -> String {
    for (k, name) in DEFAULT_LED_TO_KEY {
        if k == id {
            return name.to_string();
        }
    }
    format!("scan 0x{id:03X}")
}

#[derive(Clone)]
pub struct Keymap {
    /// `led_to_key[i]` is the key that lights LED `i`, or `None` if that LED has
    /// no key (blocker positions, ISO-only keys, …).
    pub led_to_key: [Option<KeyId>; MAIN_KEY_COUNT],
}

impl Default for Keymap {
    fn default() -> Self {
        let mut led_to_key = [None; MAIN_KEY_COUNT];
        for (i, (k, _)) in DEFAULT_LED_TO_KEY.iter().enumerate() {
            led_to_key[i] = (*k != 0).then_some(*k);
        }
        Self { led_to_key }
    }
}

#[derive(Serialize, Deserialize)]
struct KeymapFile {
    version: u32,
    /// One entry per LED index, `"0x01E"` or `null`.
    led_to_key: Vec<Option<String>>,
}

impl Keymap {
    pub fn from_slice(src: &[Option<KeyId>]) -> Self {
        let mut led_to_key = [None; MAIN_KEY_COUNT];
        for (dst, s) in led_to_key.iter_mut().zip(src) {
            *dst = s.filter(|k| (*k as usize) < KEY_SLOTS);
        }
        Self { led_to_key }
    }

    /// Missing file simply means "use the built-in guess".
    pub fn load(path: &Path) -> (Self, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => {
                return (
                    Self::default(),
                    Some(format!(
                        "could not read keymap.json ({e}) — using the built-in layout"
                    )),
                );
            }
        };
        let file: KeymapFile = match serde_json::from_str(&text) {
            Ok(f) => f,
            Err(e) => {
                return (
                    Self::default(),
                    Some(format!(
                        "keymap.json is not valid ({e}) — using the built-in layout"
                    )),
                );
            }
        };
        let parsed: Vec<Option<KeyId>> = file
            .led_to_key
            .iter()
            .map(|s| s.as_deref().and_then(parse_key_id))
            .collect();
        (Self::from_slice(&parsed), None)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let file = KeymapFile {
            version: 1,
            led_to_key: self
                .led_to_key
                .iter()
                .map(|k| k.map(|k| format!("0x{k:03X}")))
                .collect(),
        };
        let mut text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        text.push('\n');
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// How many LEDs currently have a key bound — shown in the GUI.
    pub fn bound_count(&self) -> usize {
        self.led_to_key.iter().filter(|k| k.is_some()).count()
    }
}

fn parse_key_id(s: &str) -> Option<KeyId> {
    let s = s.trim();
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u16::from_str_radix(s, 16)
        .ok()
        .filter(|k| (*k as usize) < KEY_SLOTS)
}
