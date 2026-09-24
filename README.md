# GMMK3 RGB

**Your keys sit at whatever color you pick. Press one and it flashes a second
color, then fades back. One 6 MB exe, no service, no launcher, no account.**

A lightweight standalone lighting controller for a **Glorious GMMK 3 100% ANSI** running
custom QMK firmware with the OpenRGB Raw HID protocol. Four threads, written in
Rust.

Caps Lock indicated via top right LED near knob.

### ⬇ [**Download gmmk3_rgb.zip**](https://github.com/OscarOdh/GMMK3_RGB/raw/main/dist/gmmk3_rgb.zip) · 3 MB

Unzip anywhere and run `gmmk3_rgb.exe`. No installer, no dependencies, nothing
to configure. It starts lighting the keyboard immediately.

⚠️ Requires the keyboard to be running **custom QMK firmware with OpenRGB Raw
HID**. It will not talk to stock Glorious firmware. See [Requirements](#requirements).

---

## What problem does this solve?

Keyboard RGB software has a reputation, and it's earned. The typical vendor app
is a few hundred megabytes, installs a background service and a launcher and an
updater, wants an account, phones home, and consumes measurable CPU to animate
some LEDs. OpenRGB is a huge improvement but it's a general-purpose tool for
hundreds of devices, which means a lot of abstraction between you and the one
keyboard you actually own.

This is the opposite approach: a single executable that does exactly one thing
for exactly one keyboard, with the protocol details verified against the hardware
rather than inferred from a spec.

**Why write it at all, when the keyboard has onboard effects?** Because
reactive-typing effects that live in the firmware can't be tuned, and the ones
that can be tuned need software anyway. This does the effect on the host. The app
computes each frame, works out which of 125 LEDs actually changed, and sends only
those. When you stop typing it stops sending. A still keyboard costs one atomic
read and an array compare per tick.

**Why Rust?** Because the hard parts of this are a low-level keyboard hook that
must return in microseconds or Windows drops it, and a USB endpoint that stalls if
you burst writes at it. Both are places where a garbage-collection pause or a
surprise allocation shows up as your keyboard stuttering.

> **Important prerequisite:** this requires the keyboard to be running **custom
> QMK firmware with OpenRGB Raw HID support**. It will not talk to stock Glorious
> firmware. That's the trade: you flash the keyboard once, and afterwards you own
> the lighting stack completely.

---

## Features

**Reactive typing, done on the host.** Keys sit at your chosen base color, and
each keypress flashes its LED and fades back over a configurable duration.
Because the fade is computed here rather than in firmware, the color and the
timing are both just sliders.

**60 FPS that costs nothing when idle.** The render loop diffs each frame against
what the board is already showing and writes only the differences, capped at 24
LED writes per tick so a full repaint spreads over several frames instead of
flooding the USB endpoint. Nothing changing means nothing sent.

**Independent zones.** The main key matrix, the left underglow bar, the right
underglow bar and the knob accent are four separate colors. A dim warm white
spill under a cool matrix, or each bar its own shade, or all four the same.
Whatever you pick is a colour picker away.

**Caps Lock you can see.** While Caps Lock is on, the knob LED changes color.
Tracked properly, too. See the note about `GetKeyState` below, which is the kind
of thing that looks trivial and isn't.

**Lives in the tray.** Close the window and it keeps running. Launch the exe
again and instead of starting a second copy, it tells the running one to show
itself. Optionally starts hidden.

**Sleep and resume survive.** The app listens for Windows power broadcasts and
waits 3 seconds after resume before touching USB, because the bus hasn't finished
re-enumerating yet. Reconnects cleanly instead of coming back dead.

**It refuses to fight other software.** If OpenRGB, Glorious Core, SignalRGB,
Artemis, VIA or QMK Toolbox is running, it declines to connect and tells you
which one. Windows lets several processes hold the same HID device, and when two
both write, the lighting is nondeterministic and debugging becomes impossible.
This is a correctness feature, not politeness.

**A diagnostics mode written for a human.** `--debug` runs seven plain-language
checks, each reporting what it found *and what to do about it*, then asks you
whether the keyboard actually turned green, because software can't see your
keyboard and those are different questions. Saves a full report to a text file.

**Learns your keymap if the default is wrong.** A guided mode: press each key,
and it records which scan code maps to which LED. Writes `keymap.json`. If that
file is absent, the built-in verified table is used.

**Live edits, explicit saves.** Moving a slider updates the keyboard instantly
through a channel and never touches disk. Disk is written only when you click
Save. You can experiment freely and walk away without having changed anything.

**No console window, no installer, no dependencies.** One exe plus a
`config.json`. Copy the folder anywhere.

---

## Requirements

| What | Notes |
|---|---|
| Windows 10 or 11 | Windows-only by design: Win32 keyboard hook, tray, power broadcasts |
| A Glorious GMMK 3 100% ANSI | VID `504B`, PID `320F`, interface `MI_01` |
| **Custom QMK firmware with OpenRGB Raw HID** | Non-negotiable. Stock firmware doesn't expose the protocol. |
| Rust 1.85+ *(to build)* | Edition 2024. Not needed if you just run the prebuilt exe. |

The keyboard must also **not** be in VIA mode (`Fn+O` toggles it). The app
detects VIA mode and refuses rather than misinterpreting its replies.

---

## Quick start

### Just run it

[Download `gmmk3_rgb.zip`](https://github.com/OscarOdh/GMMK3_RGB/raw/main/dist/gmmk3_rgb.zip),
unzip it anywhere, and run `gmmk3_rgb.exe`. That's the whole install: no
installer, no runtime to fetch, no registry keys.

Keep `config.json` next to the exe; that's where your colours are saved.

### Or build it yourself

```bash
git clone https://github.com/OscarOdh/GMMK3_RGB.git
cd GMMK3_RGB
cargo build --release
```

Then run it from **`dist\`**, the ready-to-go copy of exe plus config, and where
you should run it from.

Keeping it outside `target\` matters: `config.json` lives next to the exe, so a
build directory is the one place it must not be. `cargo clean` would take your
settings with it.

That clean is worth running when you're finished. `target\` reaches ~1.8 GB
(about 1.2 GB of it debug artifacts from `cargo build` and `cargo clippy`),
against ~7 MB for the project without it. Only `dist\` needs keeping.

### If nothing lights up

Run `gmmk3_rgb.exe --debug` before anything else, and read the next section
first, because most of the plausible explanations are wrong.

---

## Read this first: assumptions that are wrong

Every line below cost real debugging. If you are about to "fix" one of these,
you are about to reintroduce a bug.

### Protocol

| Natural assumption | Reality |
| --- | --- |
| `SET_MODE`'s mode byte follows the command | It is at **index 4**, behind hue/sat/value. Writing it at index 1 lands it in *hue*, so the board changes colour while the effect never moves. |
| `GET_MODE_INFO` and `SET_MODE` share a field order | They **do not**. Get is `[cmd][mode][speed][hue][sat][val]`; set is `[cmd][hue][sat][val][mode][speed][save]`. Deriving one from the other is how the above bug survived four rounds. |
| Direct mode is 0, or "the first mode" | It is **45**. `0` is `RGB_MATRIX_NONE`, which switches the board off. An easy false lead, because it looks like "the command worked but brightness is broken". |
| QMK Raw HID reports are 32 bytes (`RAW_EPSIZE`) | **This firmware uses 64.** Never hardcode it; it is read from the HID report descriptor at connect, and the app refuses to run rather than guess. |
| Bulk `DIRECT_MODE_SET_LEDS` (0x09) is the efficient way to paint | It is **deliberately unused**. See [Why SET_LEDS is not used](#why-set_leds-is-not-used) before adding it back. |
| The board has 124 LEDs | **125**, and indexes **113 and 123 are wired to nothing**. The original spec was wrong; `GET_DEVICE_INFO` reports 125 and the hardware agrees. |
| LED 96 is Right Windows | It is **Fn**, handled inside QMK, and produces no scan code. It can never light reactively. |
| Any RGB software can coexist | Windows lets several processes hold the same HID device. OpenRGB and this app will both write and neither wins, and any observation made while another is running is worthless. |

### Platform

| Natural assumption | Reality |
| --- | --- |
| `eframe::App` has `fn update(&mut self, ctx, frame)` | **Not in eframe 0.36.** The trait is `fn ui(&mut self, ui: &mut egui::Ui, frame)` plus `fn logic(&mut self, ctx, frame)`. `logic` runs even while the window is hidden; `ui` does not. That distinction is what makes hide-to-tray work. |
| `ViewportBuilder::with_visible(false)` hides the window at launch | Not reliably. The first `logic` pass re-asserts it with `ViewportCommand::Visible(false)`. |
| A message-only window can receive `WM_POWERBROADCAST` | It cannot. Broadcasts skip `HWND_MESSAGE` windows, so `input.rs` creates a real top-level window and simply never shows it. |
| The tray can live on the main or input thread | It cannot. Showing its menu blocks that thread's message pump, and blocking the pump that owns the `WH_KEYBOARD_LL` hook stalls typing **system-wide**. |
| `tray-icon`'s `common-controls-v6` feature is harmless | It makes muda import `TaskDialogIndirect`, which exists only in comctl32 v6. With no embedded manifest the loader binds v5.82 and the process dies before `main`. Do not enable it. |
| `GetKeyState(VK_CAPITAL) & 1` gives the current Caps Lock state | Only on a thread that pulls keyboard messages. It answers from a per-thread snapshot that otherwise never refreshes, and none of our threads qualify. The app reads it **once** at startup (a thread's first USER32 call snapshots the live system state) and the LL hook tracks toggles from then on. |

### Method

These are the process traps, and they wasted more time than any code bug.

- **"The write succeeded" is not "the LEDs changed."** Every write in this
  protocol returns `Ok` whether or not it did anything. Only your eyes confirm
  painting.
- **Setting the mode the keyboard is already in is indistinguishable from being
  ignored.** A test that "passed" this way hid a broken `SET_MODE` for days.
  Always verify a state change by moving to a value you are *not* already at.
- **Close other RGB software before believing anything.** A red LED "proving"
  a command worked was OpenRGB doing it in the background.
- **Read the firmware source; do not infer the protocol.** Three separate wrong
  conclusions came from reasoning about byte layouts that were a search away.
  The layouts documented here came from
  [QMK OpenRGB PR #13036](https://github.com/qmk/qmk_firmware/pull/13036).

---

## Architecture

Four threads. The split is driven by Win32 message-pump constraints, not by
taste.

```
main thread ──── eframe/winit event loop ──── gui.rs
                     │  egui::Context (clonable, Send+Sync)
                     ▼
"input"  thread ── WH_KEYBOARD_LL hook + hidden power window (own pump)
                     │  atomics only: PRESS_MS[], LAST_PRESS_MS, SUSPENDED…
                     ▼
"render" thread ── owns the HidDevice, 60 FPS loop ──── protocol.rs
                     ▲
"tray"   thread ── tray icon + menu (own pump, may block on TrackPopupMenu)
```

**Why four, in plain terms.** Windows delivers a lot of things (the tray menu,
the low-level keyboard hook, power notifications) through a per-thread *message
pump*, a loop that pulls messages and dispatches them. A thread that blocks stops
pumping. The tray menu blocks its thread for as long as it's open, which is fine
on its own thread and catastrophic on the thread that owns the keyboard hook:
Windows gives a hook callback a deadline (`LowLevelHooksTimeout`) and silently
removes the hook if it's missed, which shows up as typing stuttering
*system-wide*. So the tray gets its own thread, the hook gets its own thread, USB
gets its own thread, and the GUI keeps the main one.

Data flow:

- **GUI → render**: `mpsc::Sender<render::Msg>` behind a
  `static Mutex<Option<Sender>>`, because the tray's event handler must be
  `Sync` and `Sender` is not.
- **Hook → render**: plain atomics. The hook callback never allocates and never
  locks, because it must return well inside Windows' `LowLevelHooksTimeout` or
  the OS silently removes it and input stutters globally.
- **Render → GUI**: `Arc<Mutex<render::Status>>`, written only on state changes,
  read ~4×/s while the window is visible.

---

## File guide

### `main.rs`: entry, mode dispatch, wiring

The startup sequence, in order, and the order is deliberate.

Handles `--probe` and `--debug` *before* the single-instance guard, so
diagnostics run while the main app is up. Acquires the instance mutex, loads
config and keymap, spawns input and render threads, then hands off to eframe.
The tray is spawned inside the app creator because `set_ui_ctx` needs the
`egui::Context`.

`#![windows_subsystem = "windows"]` at the top means no console window. It also
means **`println!` goes nowhere**, so use the diagnostics report or a file.

### `protocol.rs`: the QMK OpenRGB wire format

The only file that touches `hidapi`. Every USB byte in the program goes through
here. Contains the verified command layouts, the HID report-descriptor parser
that measures packet size, and the VIA-vs-OpenRGB handshake.

Special considerations:
- `open()` refuses on three conditions before it will return a handle: a
  conflicting process, an unreadable report size, and VIA mode. All three are
  fail-closed by design, because it would rather not run than run wrong.
- `set_direct_mode()` **verifies by read-back**. Do not simplify this to a
  fire-and-forget write.
- `drain_input()` is called after writes so firmware replies cannot accumulate.

### `render.rs`: the 60 FPS loop

Owns the `Keyboard`; nothing else may touch the USB handle. Builds a 125-LED
frame, diffs it per LED against what the board is known to be showing, and
writes only the differences via `SET_SINGLE_LED`, capped at
`MAX_WRITES_PER_TICK` (24) so a full 125-LED repaint spreads over several ticks
instead of bursting.

Special considerations:
- **The write budget and the rolling `cursor` are not premature optimisation.**
  Bursting writes at this device is what stalls its endpoint.
- A write failure backs off 3 s, and after 3 consecutive failures the loop
  **halts** and waits for `Msg::Reconnect`. Hammering a stalled endpoint is what
  makes keys stick.
- Idle is genuinely idle. One atomic load (`LAST_PRESS_MS`) decides whether
  anything is decaying, so a still keyboard costs an array compare per tick.

### `input.rs`: Win32 hook and power events

One thread, one message pump, two jobs: watch every keystroke system-wide, and
listen for sleep/resume. Exposes everything through statics:
`PRESS_MS[KEY_SLOTS]`, `LAST_PRESS_MS`, `LAST_KEY_ID`/`LAST_KEY_SEQ` (keymap
learning), `CAPS_ON`, `SUSPENDED`, `RESUME_GRACE_UNTIL_MS`.

Statics and atomics rather than channels or locks, because the hook callback runs
on Windows' deadline and must not allocate or block.

Caps Lock is *tracked*, not queried (see the platform table). The hook toggles
`CAPS_ON` on the `VK_CAPITAL` key-down transition and uses a `CAPS_HELD` flag to
ignore typematic repeats, since Windows toggles on the transition only.
`seed_caps_lock()` captures the initial state at startup.

Special considerations:
- `now_ms()` returns elapsed + 1, so **0 is a valid "never pressed" sentinel**.
- The hidden window's class name is also how `instance.rs` finds a running copy.
- On resume the app waits `RESUME_GRACE_MS` (3 s) before touching USB, because
  the bus has not finished re-enumerating.

### `keymap.rs`: scan code ↔ LED index

The translation table from "a key was pressed" to "which LED to flash."

Keys are identified by **PS/2 set-1 scan code + extended flag**, not virtual key
code: VK changes with Num Lock and layout, scan codes do not. Two cases need a
synthetic id (`0x200 | vk`): Pause/Break shares scan code `0x45` with Num Lock,
and injected events can arrive with scan code 0. Hence `KEY_SLOTS = 0x300`.

`DEFAULT_LED_TO_KEY` is verified against the hardware, not guessed. Key id `0`
means "no key reaches Windows here" (LED 96, Fn).

### `gui.rs`: the main window

egui front end. Live edits go to the render thread immediately; disk is touched
only on **Save**.

Special considerations:
- Hide-to-tray relies on `logic()` running while hidden. See the eframe 0.36
  note above.
- `frames_since_show >= 2` gates close handling, because while hidden eframe
  replays the last shown frame's input and `close_requested` would read true
  forever.
- `raw_input_hook` strips key events during keymap learning so pressing Space
  does not activate a focused button.
- There is **no direct-mode control here on purpose.** An "adopt the current
  mode" button used to exist; it let a wrong value (mode 1) get saved, after
  which the app decided it was already configured and stopped trying to switch.

### `debug.rs`: the `--debug` diagnostics window

Plain-language checks with fixes, solid-colour buttons, a stress test, and a
saveable report. Written for a non-expert.

It is **not** a protocol workbench any more. Payload-shape pickers, header-order
toggles, mode sweeps and a raw hex sender all existed only while the protocol
was unknown; they became noise and a way to wedge the keyboard. Resist adding
them back.

### `conflicts.rs`: rival RGB software detection

Toolhelp process scan for OpenRGB, Glorious Core, Artemis, SignalRGB, VIA, QMK
Toolbox. `protocol::open()` refuses while any is running. This is a correctness
feature, not politeness. Shared HID access makes the lighting nondeterministic
and makes debugging impossible.

### `instance.rs`: single-instance guard

Session-local named mutex. A second launch posts `WM_APP+1` to the first
instance's hidden power window (found by class name) and exits, so
double-clicking the exe restores a tray-hidden window.

### `tray.rs`: system tray

Own thread, own pump. `UI_CTX` (a `OnceLock<egui::Context>`) lets both the tray
and the power window wake the event loop via `request_show()`.

### `config.rs`: settings and the LED map

`config.json` plus the zone constants. Read once at startup, written only on
Save. Every field has a serde default, so a partial or missing file works.

### `probe.rs`: the `--probe` fallback

Read-only, no window. Exists for the one case `--debug` cannot cover: a machine
where the GUI will not start. Writes `probe.txt`.

### `icon.rs`: procedural tray/window icon

32×32 RGBA drawn in code so there is no asset file to ship.

---

## Verified protocol reference

Everything here is confirmed against this firmware. Command ids from the QMK
OpenRGB source; field layouts confirmed by read-back or by looking at the board.

```
Transport : Raw HID, usage page 0xFF60 / usage 0x61
Device    : VID 504B  PID 320F  interface MI_01
Protocol  : OpenRGB v14   (GET_PROTOCOL_VERSION -> 01 0E ..)
Packet    : 64-byte reports (measured; QMK's stock 32 is wrong here)
```

hidapi writes take a leading report-id byte, so **firmware `data[n]` is our
`out[n+1]`**. Layouts below are in firmware indexing.

```
0x01 GET_PROTOCOL_VERSION   -> [cmd][version]              OpenRGB: 1 byte
                                                           VIA: big-endian u16
0x04 GET_MODE_INFO          -> [cmd][mode][speed][hue][sat][val]
0x07 SET_MODE               <- [cmd][hue][sat][val][mode][speed][save]
0x08 SET_SINGLE_LED         <- [cmd][led][r][g][b]
```

`SET_MODE` is sent as hue 0, sat 255, **val 255** (a zero here blanks the
board), mode 45, speed 127, **save 0** (never write EEPROM).

### Why SET_LEDS is not used

`DIRECT_MODE_SET_LEDS` (0x09) takes a start index and a count. Which order was
never settled. The firmware source said `[first][count]`, but sending that
produced a one-byte colour shift, so this firmware's build differs from the PR.

Getting it wrong is not cosmetic. With the bytes reversed, a packet starting at
LED 40 declares a count of 40, far more LEDs than a 64-byte packet holds. The
firmware reads past the end, the endpoint stops draining, and because lighting
and typing share one USB device, **key-up reports go missing and the last key
pressed repeats until the keyboard is replugged.**

`SET_SINGLE_LED` cannot overrun by construction, is confirmed correct, and for a
reactive effect is genuinely the better fit. A decay touches a handful of LEDs,
so a tick costs a handful of writes and idle costs none. The bulk path is a
performance answer to a problem this app does not have.

---

## Hardware map

Established by lighting each index individually and looking.

| LEDs | Zone |
| --- | --- |
| 0-103 | Main matrix |
| 104-112 | Left underglow, 104 at the top |
| **113** | **nothing, no LED wired** |
| 114-122 | Right underglow, 114 at the **bottom** |
| **123** | **nothing, no LED wired** |
| 124 | Knob accent |

Main matrix order is row-major, left to right, top to bottom, with the nav
cluster and numpad included in each row:

```
Row 1  (16)  LED   0.. 15   Esc F1..F12 PrtSc ScrLk Pause
Row 2  (21)  LED  16.. 36   ` 1..0 - = Bksp | Ins Home PgUp | NumLk / * -
Row 3  (21)  LED  37.. 57   Tab Q..P [ ] \  | Del End PgDn | 7 8 9 +
Row 4  (16)  LED  58.. 73   Caps A..L ; ' Enter      | 4 5 6
Row 5  (17)  LED  74.. 90   LShift Z../ RShift | Up  | 1 2 3 NumEnter
Row 6  (13)  LED  91..103   LCtrl LWin LAlt Space RAlt Fn Menu RCtrl
                            | Left Down Right | 0 .
```

The dead indexes are held at black so a frame is fully determined rather than
carrying stale bytes.

---

## config.json

Read once at startup; written only when **Save** is clicked. Moving a slider
updates the keyboard live through the channel and never touches disk.

The colours below are just the shipped defaults, one person's taste and nothing
structural. Every `zones` entry, `press_color` and the Caps Lock colour is a
picker in the GUI, so edit them there or in this file, whichever you prefer.

```json
{
  "zones": {
    "main_keys": "#00FF00",
    "left_underglow": "#001900",
    "right_underglow": "#FFF0C8",
    "knob_accent": "#000000"
  },
  "reactive_effect": { "press_color": "#0000FF", "fade_duration_sec": 0.65 },
  "caps_lock_indicator": { "enabled": true, "color": "#00FF00" },
  "start_hidden": false,
  "device": { "vid": null, "pid": null, "packet_size": null, "direct_mode": 45 }
}
```

- `vid` / `pid`: optional hard filter, only needed with several QMK devices
  attached. `"0x320F"`, `"320F"` and `12815` all parse.
- `packet_size`: `null` measures it from the HID report descriptor. Override
  only if that fails.
- `direct_mode`: the firmware effect that hands the LEDs to the host. **45** on
  this keyboard. `0` turns the board off.
- `caps_lock_indicator`: while Caps Lock is on, the knob LED (124) shows
  `color` instead of `knob_accent`. On by default. Not applied to the final
  frame at shutdown, since nothing would turn it back off afterwards.

`keymap.json` is written only by **Learn keymap…**. If it's absent, the built-in
table is used.

---

## Working in this repo

- Windows-only by design (Win32 hook, tray, power broadcasts).
- `cargo clippy --all-targets -- -D warnings` is expected to pass clean.
- Editing on the original author's machine: the Bash tool fails to fork, so use
  PowerShell. Do **not** round-trip source through
  `Get-Content | Set-Content`, because it decodes as CP1252 and re-encodes as
  UTF-8, silently mangling every non-ASCII character in the file.

---

## Diagnostics

```bash
gmmk3_rgb.exe --debug
```

Press **Run checks**. Each step reports what it found *and what to do*:

| Check | A failure means |
| --- | --- |
| Other RGB software | something else holds the connection, so close it |
| Keyboard found | not plugged in, or not running QMK with OpenRGB support |
| Connect to keyboard | in VIA mode (`Fn+O`), or held by another program |
| Lighting protocol | which protocol answered, and its version |
| Packet size | read from the keyboard's own HID descriptor |
| Hand lighting to the app | the direct-mode switch was refused |
| Set every key green | the keyboard stopped accepting data |

It then asks whether the keyboard *actually turned green*, because software
cannot see your keyboard and those are different questions. **Save report**
writes `debug.txt`: results, that answer, and the keyboard's replies to every
read-only command.

`--probe` is the read-only, no-window fallback for when the GUI will not start.

### Symptom: a key sticks and repeats, `hid_write … Overlapped I/O in progress`

The endpoint stalled. Because lighting and typing share one USB device, a
stalled OUT pipe swallows key-up reports. Historically caused by a wrong report
size or a misframed bulk write; both are now designed out. Unplug and replug to
recover, then use the stress test in `--debug` to see whether it reproduces.

---

## Deliberately not done

- **Bulk `SET_LEDS`.** See above.
- **A GUI control for `direct_mode`.** A wrong value silently disables the whole
  mode switch, so `config.json` only.
- **Layer-aware knob colour.** Possible but needs firmware work, since Fn never
  reaches the host. Either a QMK `rgb_matrix_indicators_advanced_user()` hook
  that paints LED 124 itself (simplest), or firmware sending layer changes over
  Raw HID with a tag outside the OpenRGB command range for the app to read.
