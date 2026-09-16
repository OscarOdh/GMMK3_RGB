//! `gmmk3_rgb.exe --debug` — the diagnostics window.
//!
//! Written for someone whose lighting is not working and who does not know the
//! protocol, so it answers one question — *why isn't this working?* — in plain
//! language, and produces a report they can send on.
//!
//! It is deliberately not a protocol workbench. An earlier version exposed
//! payload shapes, header orders, mode sweeps and a raw hex sender, all of
//! which existed only because the protocol was still unknown. It is known now,
//! so those controls were noise at best and a way to wedge the keyboard at
//! worst. What remains is the sequence a real diagnosis actually follows.
//!
//! Nothing is sent to the keyboard until a button is pressed.

use crate::config::{Config, LED_COUNT, Paths};
use crate::conflicts;
use crate::protocol::{self, Keyboard};
use hidapi::HidApi;
use std::fmt::Write as _;
use std::time::Instant;

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("GMMK3 RGB — diagnostics")
            .with_inner_size([680.0, 760.0])
            .with_min_inner_size([560.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native(
        "GMMK3 RGB — diagnostics",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            Ok(Box::new(DebugApp::new()))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pass,
    Fail,
    Warn,
}

impl State {
    fn mark(self) -> &'static str {
        match self {
            Self::Pass => "OK",
            Self::Fail => "FAILED",
            Self::Warn => "CHECK",
        }
    }

    fn colour(self) -> egui::Color32 {
        match self {
            Self::Pass => egui::Color32::from_rgb(0x3D, 0xDC, 0x6B),
            Self::Fail => egui::Color32::from_rgb(0xD0, 0x4A, 0x4A),
            Self::Warn => egui::Color32::from_rgb(0xE0, 0x9B, 0x3A),
        }
    }
}

/// One step of the diagnosis. `fix` is what the reader should actually do,
/// which is the part that makes this useful to someone who does not know the
/// protocol — a bare "FAILED" helps nobody.
struct Check {
    name: String,
    state: State,
    detail: String,
    fix: String,
}

struct DebugApp {
    api: Option<HidApi>,
    kb: Option<Keyboard>,
    checks: Vec<Check>,
    ran: bool,
    /// The reader's answer to "did the keyboard change colour?", which is the
    /// one result no amount of software can confirm on its own.
    saw_colour: Option<bool>,
    log: Vec<String>,
    started: Instant,
}

impl DebugApp {
    fn new() -> Self {
        Self {
            api: None,
            kb: None,
            checks: Vec::new(),
            ran: false,
            saw_colour: None,
            log: Vec::new(),
            started: Instant::now(),
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        let stamp = self.started.elapsed().as_secs_f64();
        self.log.push(format!("[{stamp:7.3}] {}", text.into()));
        if self.log.len() > 1000 {
            self.log.drain(..200);
        }
    }

    fn add(&mut self, name: &str, state: State, detail: impl Into<String>, fix: impl Into<String>) {
        let (detail, fix) = (detail.into(), fix.into());
        self.say(format!("{} [{}] {detail}", name, state.mark()));
        self.checks.push(Check {
            name: name.to_string(),
            state,
            detail,
            fix,
        });
    }

    /// The whole diagnosis, in the order a person would actually work through
    /// it: rule out other software, find the keyboard, check which protocol is
    /// listening, then try to drive it.
    fn run_checks(&mut self) {
        self.checks.clear();
        self.saw_colour = None;
        self.ran = true;
        self.kb = None;

        let rivals = conflicts::running();
        if rivals.is_empty() {
            self.add(
                "Other RGB software",
                State::Pass,
                "none running",
                String::new(),
            );
        } else {
            self.add(
                "Other RGB software",
                State::Fail,
                format!("{} is running", rivals.join(", ")),
                "Close it and run these checks again. It controls the same keyboard \
                 connection, so while it is open the two programs fight over the lighting.",
            );
            return;
        }

        if self.api.is_none() {
            match HidApi::new() {
                Ok(api) => self.api = Some(api),
                Err(e) => {
                    self.add(
                        "USB support",
                        State::Fail,
                        format!("could not start hidapi: {e}"),
                        "This is a Windows-level problem rather than a keyboard one. A reboot \
                         is the usual fix.",
                    );
                    return;
                }
            }
        }
        // Each borrow of `api` is scoped, so `add` can take `&mut self` freely
        // between steps.
        let found = {
            let api = self.api.as_ref().expect("initialised above");
            protocol::candidates(api).len()
        };
        if found == 0 {
            self.add(
                "Keyboard found",
                State::Fail,
                "no QMK lighting connection detected",
                "Check the keyboard is plugged in. If it is, it may not be running the QMK \
                 firmware with OpenRGB support — this app cannot drive stock GMMK firmware.",
            );
            return;
        }
        self.add(
            "Keyboard found",
            State::Pass,
            format!("{found} QMK lighting connection(s)"),
            String::new(),
        );

        let paths = Paths::discover();
        let (cfg, _) = Config::load(&paths.config);
        let opened = {
            let api = self.api.as_mut().expect("initialised above");
            protocol::open(api, &cfg.device)
        };
        let kb = match opened {
            Ok(kb) => kb,
            Err(e) => {
                // `open` already refuses on VIA mode and names the cause, so
                // its message is the useful one to show.
                let via = e.contains("VIA mode");
                self.add(
                    "Connect to keyboard",
                    State::Fail,
                    e,
                    if via {
                        "Press Fn+O on the keyboard to switch it out of VIA mode, then run \
                         these checks again."
                    } else {
                        "Close any other lighting software and try again."
                    },
                );
                return;
            }
        };

        self.add(
            "Connect to keyboard",
            State::Pass,
            format!("{} ({:04X}:{:04X})", kb.name, kb.vid, kb.pid),
            String::new(),
        );
        self.add(
            "Lighting protocol",
            State::Pass,
            format!("OpenRGB mode, {}", kb.handshake),
            String::new(),
        );
        self.add(
            "Packet size",
            State::Pass,
            format!("{} bytes ({})", kb.packet_size, kb.packet_size_source),
            String::new(),
        );

        match kb.set_direct_mode(cfg.device.direct_mode) {
            Ok(note) => self.add("Hand lighting to the app", State::Pass, note, String::new()),
            Err(e) => self.add(
                "Hand lighting to the app",
                State::Fail,
                e,
                format!(
                    "The keyboard would not switch to lighting mode {}. If your firmware \
                     differs, change device.direct_mode in config.json.",
                    cfg.device.direct_mode
                ),
            ),
        }

        match paint(&kb, [0, 255, 0]) {
            Ok(()) => self.add(
                "Set every key green",
                State::Warn,
                "sent — look at the keyboard",
                "Answer the question below. Software cannot see your keyboard, so this is \
                 the one step only you can confirm.",
            ),
            Err(e) => self.add(
                "Set every key green",
                State::Fail,
                e,
                "The keyboard stopped accepting data. Unplug it, plug it back in, and run \
                 these checks again.",
            ),
        }

        self.kb = Some(kb);
    }

    fn paint_now(&mut self, label: &str, rgb: [u8; 3]) {
        let Some(kb) = &self.kb else {
            self.say(format!("{label}: not connected — run the checks first"));
            return;
        };
        let t = Instant::now();
        match paint(kb, rgb) {
            Ok(()) => self.say(format!(
                "{label}: sent in {:.0} ms",
                t.elapsed().as_secs_f64() * 1000.0
            )),
            Err(e) => self.say(format!("{label}: FAILED — {e}")),
        }
    }

    /// The failure this app has hit hardest is not a wrong colour, it is the
    /// keyboard's USB connection stalling under a stream of writes and taking
    /// key-up events with it. Worth being able to reproduce on demand.
    fn stress(&mut self) {
        let Some(kb) = &self.kb else {
            self.say("stress test: not connected — run the checks first");
            return;
        };
        let (mut worst, mut failed_at) = (0.0f64, None);
        for i in 0..300u32 {
            let shade = (i % 256) as u8;
            let t = Instant::now();
            if let Err(e) = kb.set_single_led(0, [shade, 255 - shade, 0]) {
                failed_at = Some((i, e));
                break;
            }
            worst = worst.max(t.elapsed().as_secs_f64() * 1000.0);
            kb.drain_input();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        match failed_at {
            None => self.say(format!(
                "stress test: 300 writes all succeeded, slowest {worst:.1} ms — the \
                 connection keeps up"
            )),
            Some((i, e)) => self.say(format!(
                "stress test: FAILED on write {i} of 300 — {e}. Unplug and replug the keyboard."
            )),
        }
    }

    fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "GMMK3 RGB — diagnostics report\n");
        for c in &self.checks {
            let _ = writeln!(out, "[{}] {} — {}", c.state.mark(), c.name, c.detail);
            if !c.fix.is_empty() {
                let _ = writeln!(out, "       {}", c.fix);
            }
        }
        let _ = writeln!(
            out,
            "\nKeyboard visibly changed colour: {}",
            match self.saw_colour {
                Some(true) => "yes",
                Some(false) => "no",
                None => "not answered",
            }
        );

        // The firmware's own answers, which are what any further diagnosis
        // would start from.
        if let Some(kb) = &self.kb {
            let _ = writeln!(out, "\n--- device replies ---");
            for (command, name) in protocol::GET_COMMANDS {
                match kb.query(command) {
                    Ok(reply) => {
                        let hex: Vec<String> = reply[..reply.len().min(24)]
                            .iter()
                            .map(|b| format!("{b:02X}"))
                            .collect();
                        let _ = writeln!(out, "{name}: {}", hex.join(" "));
                    }
                    Err(e) => {
                        let _ = writeln!(out, "{name}: {e}");
                    }
                }
                kb.drain_input();
            }
        }

        let _ = writeln!(out, "\n--- log ---");
        for line in &self.log {
            let _ = writeln!(out, "{line}");
        }
        out
    }
}

/// Paint every LED one at a time. `SET_SINGLE_LED` is the write this firmware
/// is known to honour, and it cannot overrun the packet, so it is what both the
/// app and this tool use.
fn paint(kb: &Keyboard, rgb: [u8; 3]) -> Result<(), String> {
    for led in 0..LED_COUNT {
        kb.set_single_led(led, rgb)?;
    }
    kb.drain_input();
    Ok(())
}

impl eframe::App for DebugApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("GMMK3 RGB diagnostics");
            ui.label(
                egui::RichText::new(
                    "Checks why the lighting is not working and tells you what to do about it. \
                     Nothing is sent to your keyboard until you press a button.",
                )
                .weak(),
            );
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new("  Run checks  ").heading(),
                    ))
                    .clicked()
                {
                    self.run_checks();
                }
                ui.label(
                    egui::RichText::new("Close OpenRGB and any other lighting software first.")
                        .small()
                        .weak(),
                );
            });

            ui.add_space(10.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.results(ui);
                    self.tools(ui);
                    self.log_view(ui);
                });
        });
    }
}

impl DebugApp {
    fn results(&mut self, ui: &mut egui::Ui) {
        if !self.ran {
            return;
        }
        ui.label(egui::RichText::new("RESULTS").small().weak());
        for check in &self.checks {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(check.state.colour(), format!("{:>7}", check.state.mark()));
                ui.label(&check.name);
                ui.label(egui::RichText::new(format!("— {}", check.detail)).weak());
            });
            if !check.fix.is_empty() {
                ui.indent(check.name.clone(), |ui| {
                    ui.label(egui::RichText::new(&check.fix).small());
                });
            }
        }

        // Only ask once the checks got far enough to have painted something.
        let painted = self
            .checks
            .iter()
            .any(|c| c.name == "Set every key green" && c.state != State::Fail);
        if painted {
            ui.add_space(8.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Did every key on the keyboard turn green?").heading(),
                );
                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        self.saw_colour = Some(true);
                        self.say("user reports: keyboard turned green");
                    }
                    if ui.button("No").clicked() {
                        self.saw_colour = Some(false);
                        self.say("user reports: keyboard did NOT turn green");
                    }
                });
                match self.saw_colour {
                    Some(true) => {
                        ui.colored_label(
                            State::Pass.colour(),
                            "Then the app can drive your lighting. If the main window still \
                             looks wrong, the problem is in its settings rather than the \
                             connection.",
                        );
                    }
                    Some(false) => {
                        ui.colored_label(
                            State::Fail.colour(),
                            "Every check passed but nothing changed on the keyboard. Save the \
                             report below and send it on — it contains what the keyboard said \
                             about itself, which is where a fix would start.",
                        );
                    }
                    None => {}
                }
            });
        }
        ui.add_space(14.0);
    }

    fn tools(&mut self, ui: &mut egui::Ui) {
        let connected = self.kb.is_some();
        ui.label(egui::RichText::new("TRY THE LIGHTS").small().weak());
        ui.label(
            egui::RichText::new(
                "Sets the whole keyboard to one colour, so you can see for yourself whether \
                 the app is in control.",
            )
            .small()
            .weak(),
        );
        ui.horizontal(|ui| {
            for (label, rgb) in [
                ("Red", [255u8, 0, 0]),
                ("Green", [0, 255, 0]),
                ("Blue", [0, 0, 255]),
                ("White", [255, 255, 255]),
                ("Off", [0, 0, 0]),
            ] {
                if ui
                    .add_enabled(connected, egui::Button::new(label))
                    .clicked()
                {
                    self.paint_now(label, rgb);
                }
            }
        });
        if !connected {
            ui.label(
                egui::RichText::new("Run the checks first to connect.")
                    .small()
                    .weak(),
            );
        }

        ui.add_space(14.0);
        ui.label(egui::RichText::new("STRESS TEST").small().weak());
        ui.label(
            egui::RichText::new(
                "Sends 300 rapid updates. Use this if keys ever stick or repeat by themselves \
                 while the app is running: that happens when the keyboard's connection stalls \
                 under load, and this reproduces it on demand. If it fails, unplug and replug \
                 the keyboard.",
            )
            .small()
            .weak(),
        );
        if ui
            .add_enabled(connected, egui::Button::new("Run stress test"))
            .clicked()
        {
            self.stress();
        }
        ui.add_space(14.0);
    }

    fn log_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("REPORT").small().weak());
            if ui
                .button("Save report")
                .on_hover_text("Writes debug.txt next to the app.")
                .clicked()
            {
                let path = Paths::discover().config.with_file_name("debug.txt");
                let text = self.report();
                match std::fs::write(&path, text) {
                    Ok(()) => self.say(format!("report saved to {}", path.display())),
                    Err(e) => self.say(format!("could not save report: {e}")),
                }
            }
            if ui.button("Copy report").clicked() {
                let text = self.report();
                ui.ctx().copy_text(text);
                self.say("report copied to the clipboard");
            }
        });
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.log.is_empty() {
                        ui.label(egui::RichText::new("Nothing yet.").small().weak());
                    }
                    for line in &self.log {
                        ui.label(egui::RichText::new(line).small().monospace());
                    }
                });
        });
    }
}
