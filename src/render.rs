//! The 60 FPS render loop. Owns the USB handle; nothing else touches it.
//!
//! The loop builds a frame, compares it with the last one that went out, and
//! only writes when something changed — so with no keys decaying the thread
//! wakes 60 times a second, does an array compare, and goes back to sleep.

use crate::config::{
    Config, FRAME_BYTES, KNOB_END, KNOB_START, LED_COUNT, LEFT_UG_END, LEFT_UG_START, MAIN_END,
    MAIN_START, RIGHT_UG_END, RIGHT_UG_START,
};
use crate::input;
use crate::keymap::Keymap;
use crate::protocol::{self, Keyboard};
use hidapi::HidApi;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FRAME_INTERVAL: Duration = Duration::from_micros(16_667);
/// Re-assert the override and repaint everything this often, so the board and
/// our idea of it cannot drift apart.
const KEEPALIVE_MS: u64 = 5_000;
/// Most LEDs to write in one tick, so a full repaint spreads over a few ticks
/// instead of bursting 125 back-to-back writes at the device.
const MAX_WRITES_PER_TICK: usize = 24;
const CONNECT_BACKOFF_START_MS: u64 = 500;
const CONNECT_BACKOFF_MAX_MS: u64 = 5_000;
/// A device that just rejected a write needs longer than a plain reconnect.
const WRITE_FAILURE_COOLDOWN_MS: u64 = 3_000;
/// Consecutive write failures before we stop trying and wait to be told to
/// retry. Hammering an endpoint that has stopped draining is what wedges the
/// keyboard, so the safe move is to stand down and say so.
const MAX_WRITE_FAILURES: u32 = 3;

pub enum Msg {
    Config(Config),
    /// Boxed — a `Keymap` is a few hundred bytes and would otherwise set the
    /// size of every message that goes through the channel.
    Keymap(Box<Keymap>),
    /// `Some(led)` lights that LED white and blacks out everything else, for
    /// the Learn Keymap panel. `None` returns to normal rendering.
    Probe(Option<u16>),
    /// Clear the write-failure brake and try the device again.
    Reconnect,
    Shutdown,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub connected: bool,
    pub suspended: bool,
    /// Stopped after repeated write failures; waiting for `Msg::Reconnect`.
    pub halted: bool,
    pub device: String,
    pub handshake: String,
    /// Empty when all is well.
    pub message: String,
}

/// The GUI and the tray both post here. A `Mutex<Option<Sender>>` rather than a
/// bare `Sender` because the tray's event handler must be `Sync`.
static TX: Mutex<Option<Sender<Msg>>> = Mutex::new(None);
static DONE: AtomicBool = AtomicBool::new(false);
static FRAMES_SENT: AtomicU64 = AtomicU64::new(0);

pub fn send(msg: Msg) {
    if let Ok(guard) = TX.lock()
        && let Some(tx) = guard.as_ref()
    {
        let _ = tx.send(msg);
    }
}

/// True once the loop has written its final frame and stopped.
pub fn is_done() -> bool {
    DONE.load(Ordering::SeqCst)
}

pub fn frames_sent() -> u64 {
    FRAMES_SENT.load(Ordering::Relaxed)
}

pub fn spawn(cfg: Config, keymap: Keymap, status: Arc<Mutex<Status>>) {
    let (tx, rx) = mpsc::channel();
    if let Ok(mut guard) = TX.lock() {
        *guard = Some(tx);
    }
    let _ = std::thread::Builder::new()
        .name("render".to_string())
        .spawn(move || run(rx, cfg, keymap, status));
}

fn set_status(slot: &Arc<Mutex<Status>>, next: Status) {
    if let Ok(mut cur) = slot.lock() {
        *cur = next;
    }
}

fn run(rx: Receiver<Msg>, mut cfg: Config, mut keymap: Keymap, status: Arc<Mutex<Status>>) {
    let mut api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            set_status(
                &status,
                Status {
                    message: format!("could not initialise hidapi: {e}"),
                    ..Default::default()
                },
            );
            DONE.store(true, Ordering::SeqCst);
            return;
        }
    };

    let mut dev: Option<Keyboard> = None;
    let mut probe: Option<u16> = None;
    let mut frame = [0u8; FRAME_BYTES];
    let mut last_frame = [0u8; FRAME_BYTES];
    // Which LEDs the board is known to be showing correctly.
    let mut synced = [false; LED_COUNT];
    // Rolling start for the write scan, so no LED can starve behind a busy one.
    let mut cursor = 0usize;
    let mut built = false;
    let mut last_sent_ms = 0u64;
    let mut next_connect_ms = 0u64;
    let mut backoff_ms = CONNECT_BACKOFF_START_MS;
    // Set whenever something other than the passage of time changed the frame.
    let mut dirty = true;
    let mut was_decaying = false;
    // Last Caps Lock state rendered; a change marks the frame dirty.
    let mut caps_last = input::CAPS_ON.load(Ordering::Relaxed);
    // `Some(suspended)` while parked, so the status is written once per change.
    let mut parked_as: Option<bool> = None;
    let mut applied_mode: Option<u8> = None;
    let mut write_failures: u32 = 0;
    let mut halted = false;

    loop {
        let tick_start = Instant::now();

        let mut shutting_down = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Config(c) => {
                    dirty |= c != cfg;
                    cfg = c;
                }
                Msg::Keymap(k) => {
                    keymap = *k;
                    dirty = true;
                }
                Msg::Probe(p) => {
                    dirty |= p != probe;
                    probe = p;
                }
                Msg::Reconnect => {
                    halted = false;
                    write_failures = 0;
                    backoff_ms = CONNECT_BACKOFF_START_MS;
                    next_connect_ms = 0;
                }
                Msg::Shutdown => shutting_down = true,
            }
        }

        let now = input::now_ms();

        if shutting_down {
            // Leave the board on a clean static frame rather than mid-decay,
            // via the same single-LED path the loop uses.
            if let Some(kb) = &dev {
                build_frame(&mut frame, &cfg, &keymap, now, None, true);
                for led in 0..LED_COUNT {
                    let base = led * 3;
                    let rgb = [frame[base], frame[base + 1], frame[base + 2]];
                    if kb.set_single_led(led, rgb).is_err() {
                        break;
                    }
                }
            }
            DONE.store(true, Ordering::SeqCst);
            return;
        }

        let suspended = input::SUSPENDED.load(Ordering::SeqCst);
        let in_resume_grace = now < input::RESUME_GRACE_UNTIL_MS.load(Ordering::SeqCst);
        if suspended || in_resume_grace {
            if dev.is_some() {
                dev = None; // dropping the handle closes it
                synced = [false; LED_COUNT];
            }
            // Report once per state change, not five times a second for the
            // whole time the machine is asleep.
            if parked_as != Some(suspended) {
                parked_as = Some(suspended);
                set_status(
                    &status,
                    Status {
                        suspended: true,
                        message: if suspended {
                            "system suspended — USB handle released".to_string()
                        } else {
                            "waiting for the USB bus to re-enumerate…".to_string()
                        },
                        ..Default::default()
                    },
                );
            }
            next_connect_ms = now;
            backoff_ms = CONNECT_BACKOFF_START_MS;
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        parked_as = None;

        if dev.is_none() {
            if halted {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            if now >= next_connect_ms {
                match protocol::open(&mut api, &cfg.device) {
                    Ok(kb) => {
                        // This is the step that selecting "Direct" in OpenRGB
                        // performs. It is verified rather than assumed,
                        // because it had been failing silently and leaving the
                        // board on whatever effect it was already showing.
                        let mode_outcome = kb.set_direct_mode(cfg.device.direct_mode);
                        let mode_note = match &mode_outcome {
                            Ok(_) => String::new(),
                            Err(e) => e.clone(),
                        };
                        applied_mode = Some(cfg.device.direct_mode);
                        set_status(
                            &status,
                            Status {
                                connected: true,
                                device: format!("{} ({:04X}:{:04X})", kb.name, kb.vid, kb.pid),
                                handshake: format!(
                                    "{}  ·  {}-byte packets ({})  ·  mode {} {}",
                                    kb.handshake,
                                    kb.packet_size,
                                    kb.packet_size_source,
                                    cfg.device.direct_mode,
                                    match &mode_outcome {
                                        Ok(shape) => format!("({shape})"),
                                        Err(_) => "NOT SET".to_string(),
                                    }
                                ),
                                message: mode_note,
                                ..Default::default()
                            },
                        );
                        dev = Some(kb);
                        synced = [false; LED_COUNT];
                        backoff_ms = CONNECT_BACKOFF_START_MS;
                    }
                    Err(e) => {
                        set_status(
                            &status,
                            Status {
                                message: e,
                                ..Default::default()
                            },
                        );
                        next_connect_ms = now + backoff_ms;
                        backoff_ms = (backoff_ms * 2).min(CONNECT_BACKOFF_MAX_MS);
                    }
                }
            }
            if dev.is_none() {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        }

        // `Reload from disk` picked up a different direct-mode id: re-assert it.
        // (The GUI has no control for this; config.json is the only way in.)
        if applied_mode != Some(cfg.device.direct_mode)
            && let Some(kb) = &dev
        {
            let _ = kb.set_direct_mode(cfg.device.direct_mode);
            applied_mode = Some(cfg.device.direct_mode);
            synced = [false; LED_COUNT];
        }

        // One atomic load tells us whether any key is still fading. When the
        // answer is no and nothing else changed, the frame cannot differ from
        // the last one, so the whole build is skipped and the tick costs
        // almost nothing.
        let fade_ms = fade_ms(&cfg);
        let last_press = input::LAST_PRESS_MS.load(Ordering::Relaxed);
        let decaying =
            probe.is_none() && last_press != 0 && now.saturating_sub(last_press) < fade_ms;
        // Caps Lock changes the knob LED; treat a toggle like any other edit.
        let caps_on = input::CAPS_ON.load(Ordering::Relaxed);
        if caps_on != caps_last {
            caps_last = caps_on;
            dirty = true;
        }
        if dirty || decaying || was_decaying || !built {
            build_frame(&mut frame, &cfg, &keymap, now, probe, false);
            dirty = false;
            built = true;
        }
        was_decaying = decaying;

        // Repaint everything periodically so the board and our idea of it
        // cannot drift apart.
        if now.saturating_sub(last_sent_ms) >= KEEPALIVE_MS {
            synced = [false; LED_COUNT];
            last_sent_ms = now;
        }

        if let Some(kb) = &dev {
            // Painted one LED at a time with SET_SINGLE_LED, whose layout —
            // [cmd][led][r][g][b] — is the only write in this protocol that has
            // been confirmed against the hardware: every LED index in
            // config.rs was mapped with it.
            //
            // SET_LEDS is deliberately not used here. Its header size was never
            // pinned down, and getting it wrong does not merely paint wrong
            // colours: a misread count makes the firmware read past the end of
            // the packet, which stalls the endpoint and costs key-up events.
            // Single-LED writes cannot overrun by construction and have never
            // wedged the board.
            //
            // For a reactive effect they are also the better fit: a decay
            // touches a handful of LEDs, so a tick costs a handful of writes,
            // and idle costs none. The budget spreads a full repaint over a few
            // ticks instead of bursting 125 writes, and the rolling cursor
            // keeps any one LED from starving.
            let mut written = 0usize;
            let mut failure = None;
            for step in 0..LED_COUNT {
                if written >= MAX_WRITES_PER_TICK {
                    break;
                }
                let led = (cursor + step) % LED_COUNT;
                let span = led * 3..led * 3 + 3;
                if synced[led] && frame[span.clone()] == last_frame[span.clone()] {
                    continue;
                }
                let rgb = [
                    frame[span.start],
                    frame[span.start + 1],
                    frame[span.start + 2],
                ];
                match kb.set_single_led(led, rgb) {
                    Ok(()) => {
                        last_frame[span].copy_from_slice(&rgb);
                        synced[led] = true;
                        written += 1;
                        cursor = (led + 1) % LED_COUNT;
                    }
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }

            if written > 0 {
                // The firmware answers some commands; consume anything it sent
                // so replies cannot pile up behind us.
                kb.drain_input();
                write_failures = 0;
                FRAMES_SENT.fetch_add(1, Ordering::Relaxed);
            }

            if let Some(e) = failure {
                // A failed write usually means the endpoint stopped draining.
                // Retrying straight away is how you wedge the keyboard's USB
                // pipeline and start losing key-up events, so back off hard and
                // give up entirely after a few tries rather than hammering a
                // device that is not answering.
                dev = None;
                synced = [false; LED_COUNT];
                write_failures += 1;
                let giving_up = write_failures >= MAX_WRITE_FAILURES;
                halted = giving_up;
                next_connect_ms = now + WRITE_FAILURE_COOLDOWN_MS;
                set_status(
                    &status,
                    Status {
                        halted: giving_up,
                        message: if giving_up {
                            format!(
                                "the keyboard stopped accepting writes ({e}). Stopped after \
                                 {write_failures} attempts — unplug and replug it, then press Reconnect."
                            )
                        } else {
                            format!("lost the keyboard ({e}) — retrying in 3 s…")
                        },
                        ..Default::default()
                    },
                );
            }
        }

        let elapsed = tick_start.elapsed();
        if elapsed < FRAME_INTERVAL {
            std::thread::sleep(FRAME_INTERVAL - elapsed);
        }
    }
}

/// Paint one zone in a single colour.
fn fill(frame: &mut [u8; FRAME_BYTES], leds: std::ops::Range<usize>, rgb: [u8; 3]) {
    let (pixels, _) = frame[leds.start * 3..leds.end * 3].as_chunks_mut::<3>();
    pixels.fill(rgb);
}

fn put(frame: &mut [u8; FRAME_BYTES], led: usize, rgb: [u8; 3]) {
    frame[led * 3..led * 3 + 3].copy_from_slice(&rgb);
}

fn lerp(from: u8, to: u8, t: f32) -> u8 {
    (from as f32 + (to as f32 - from as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn fade_ms(cfg: &Config) -> u64 {
    (cfg.reactive_effect.fade_duration_sec.max(0.0) * 1000.0) as u64
}

/// Render into `frame`, overwriting every byte. `static_only` skips the
/// reactive pass — used for the final frame on exit.
fn build_frame(
    frame: &mut [u8; FRAME_BYTES],
    cfg: &Config,
    keymap: &Keymap,
    now: u64,
    probe: Option<u16>,
    static_only: bool,
) {
    // Learn Keymap: one white LED, everything else dark, so there is no doubt
    // about which position is being asked for.
    if let Some(led) = probe {
        frame.fill(0);
        let led = led as usize;
        if led < LED_COUNT {
            put(frame, led, [255, 255, 255]);
        }
        return;
    }

    let main = cfg.zones.main_keys.0;
    fill(frame, MAIN_START..MAIN_END, main);
    fill(
        frame,
        LEFT_UG_START..LEFT_UG_END,
        cfg.zones.left_underglow.0,
    );
    fill(
        frame,
        RIGHT_UG_START..RIGHT_UG_END,
        cfg.zones.right_underglow.0,
    );
    // The knob doubles as a Caps Lock indicator. Not applied to the final frame
    // on shutdown: once the app stops, nothing would turn it back off, and a
    // stale indicator is worse than none.
    let caps = &cfg.caps_lock_indicator;
    let knob = if caps.enabled && !static_only && input::CAPS_ON.load(Ordering::Relaxed) {
        caps.color.0
    } else {
        cfg.zones.knob_accent.0
    };
    fill(frame, KNOB_START..KNOB_END, knob);
    for led in crate::config::UNUSED_LEDS {
        put(frame, led, [0, 0, 0]);
    }

    let fade_ms = fade_ms(cfg);
    if static_only || fade_ms == 0 {
        return;
    }
    let press = cfg.reactive_effect.press_color.0;

    for led in MAIN_START..MAIN_END {
        let Some(key) = keymap.led_to_key[led] else {
            continue;
        };
        let Some(slot) = input::PRESS_MS.get(key as usize) else {
            continue;
        };
        let pressed_at = slot.load(Ordering::Relaxed);
        if pressed_at == 0 || now < pressed_at {
            continue;
        }
        let age = now - pressed_at;
        if age >= fade_ms {
            continue;
        }
        let t = age as f32 / fade_ms as f32;
        put(
            frame,
            led,
            [
                lerp(press[0], main[0], t),
                lerp(press[1], main[1], t),
                lerp(press[2], main[2], t),
            ],
        );
    }
}
