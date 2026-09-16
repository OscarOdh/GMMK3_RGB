//! The egui front end. Every edit is pushed to the render thread immediately;
//! the disk is only touched when Save is clicked.

use crate::config::{Config, HexColor, MAIN_KEY_COUNT, Paths};
use crate::input;
use crate::keymap::{self, KeyId, Keymap};
use crate::render::{self, Status};
use crate::tray;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const WINDOW_TITLE: &str = "GMMK3 RGB Controller";

/// Holding a key must not race through several LEDs. Only repeats of the key
/// just recorded are swallowed, and the timer is refreshed on every swallowed
/// repeat, so a key held down stays swallowed however long it is held — while a
/// different key is always accepted immediately.
const LEARN_REPEAT_GUARD: Duration = Duration::from_millis(700);

struct Learn {
    idx: usize,
    captured: Vec<Option<KeyId>>,
    seq: u64,
    last_id: Option<KeyId>,
    advanced_at: Instant,
}

impl Learn {
    fn new(keymap: &Keymap) -> Self {
        Self {
            idx: 0,
            captured: keymap.led_to_key.to_vec(),
            seq: input::LAST_KEY_SEQ.load(Ordering::Relaxed),
            last_id: None,
            advanced_at: Instant::now(),
        }
    }

    fn finished(&self) -> bool {
        self.idx >= MAIN_KEY_COUNT
    }

    fn sync_probe(&self) {
        let probe = if self.finished() {
            None
        } else {
            Some(self.idx as u16)
        };
        render::send(render::Msg::Probe(probe));
    }

    /// Move the cursor by hand (Back / Skip / Restart). Clears the repeat guard
    /// and swallows any keystroke already in flight so the click itself cannot
    /// be mistaken for an answer.
    fn jump_to(&mut self, idx: usize) {
        self.idx = idx.min(MAIN_KEY_COUNT);
        self.last_id = None;
        self.advanced_at = Instant::now();
        self.seq = input::LAST_KEY_SEQ.load(Ordering::Relaxed);
        self.sync_probe();
    }
}

pub struct App {
    paths: Paths,
    cfg: Config,
    /// Last config handed to the render thread.
    pushed_cfg: Config,
    /// Last config written to disk, for the "unsaved changes" hint.
    saved_cfg: Config,
    keymap: Keymap,
    status: Arc<Mutex<Status>>,
    notice: Option<(Instant, String)>,
    warning: Option<String>,
    learn: Option<Learn>,
    hidden: bool,
    /// Whether the initial hidden-on-launch state has been enforced yet.
    initial_visibility_applied: bool,
    frames_since_show: u32,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Paths,
        cfg: Config,
        keymap: Keymap,
        warning: Option<String>,
        status: Arc<Mutex<Status>>,
    ) -> Self {
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        Self {
            paths,
            hidden: cfg.start_hidden,
            pushed_cfg: cfg.clone(),
            saved_cfg: cfg.clone(),
            cfg,
            keymap,
            status,
            notice: None,
            warning,
            learn: None,
            initial_visibility_applied: false,
            frames_since_show: 0,
        }
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((Instant::now(), text.into()));
    }

    fn push_if_changed(&mut self) {
        if self.cfg != self.pushed_cfg {
            self.pushed_cfg = self.cfg.clone();
            render::send(render::Msg::Config(self.cfg.clone()));
        }
    }

    fn cancel_learn(&mut self) {
        if self.learn.take().is_some() {
            render::send(render::Msg::Probe(None));
        }
    }

    /// Consume a keystroke captured by the low-level hook, if one arrived.
    fn poll_learn(&mut self) {
        let Some(learn) = self.learn.as_mut() else {
            return;
        };
        if learn.finished() {
            return;
        }
        let seq = input::LAST_KEY_SEQ.load(Ordering::Relaxed);
        if seq == learn.seq {
            return;
        }
        learn.seq = seq;

        let id = input::LAST_KEY_ID.load(Ordering::Relaxed) as KeyId;
        if learn.last_id == Some(id) && learn.advanced_at.elapsed() < LEARN_REPEAT_GUARD {
            learn.advanced_at = Instant::now(); // still held — keep swallowing
            return;
        }

        // A key can only drive one LED, so clear any earlier claim on it.
        for slot in learn.captured.iter_mut() {
            if *slot == Some(id) {
                *slot = None;
            }
        }
        learn.captured[learn.idx] = Some(id);
        learn.idx += 1;
        learn.last_id = Some(id);
        learn.advanced_at = Instant::now();
        learn.sync_probe();
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // While learning, keystrokes belong to the hook, not to the widgets —
        // otherwise pressing Space would activate whichever button has focus.
        if self.learn.is_some() {
            raw_input
                .events
                .retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // `ViewportBuilder::with_visible(false)` alone does not reliably keep
        // the window down on this platform — it still appeared on launch — so
        // the first pass re-asserts it as a viewport command.
        if !self.initial_visibility_applied {
            self.initial_visibility_applied = true;
            if self.hidden {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        }

        if tray::SHOW_REQUEST.swap(false, Ordering::SeqCst) {
            self.hidden = false;
            self.frames_since_show = 0;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // While hidden, eframe replays the input of the last shown frame, so
        // `close_requested` would still read true. The frame counter makes sure
        // we only act on a close that this window actually just received.
        if !self.hidden
            && self.frames_since_show >= 2
            && ctx.input(|i| i.viewport().close_requested())
        {
            self.hidden = true;
            self.cancel_learn();
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames_since_show = self.frames_since_show.saturating_add(1);
        self.poll_learn();

        egui::CentralPanel::default().show(ui, |ui| {
            if self.learn.is_some() {
                self.learn_ui(ui);
            } else {
                self.main_ui(ui);
            }
        });

        if let Some((at, _)) = &self.notice
            && at.elapsed() > Duration::from_secs(4)
        {
            self.notice = None;
        }

        self.push_if_changed();

        // Learning needs a tight loop to catch keystrokes; otherwise a lazy
        // tick is enough to refresh the connection status.
        if self.learn.is_some() {
            ui.ctx().request_repaint();
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }
}

// ---------------------------------------------------------------------------
// Main panel
// ---------------------------------------------------------------------------

impl App {
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("GMMK3 RGB");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Hide to tray").clicked() {
                    self.hidden = true;
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
            });
        });

        self.status_ui(ui);
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new("ZONES").small().weak());
                egui::Grid::new("zones")
                    .num_columns(3)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        color_row(ui, "Main keys", "LED 0–103", &mut self.cfg.zones.main_keys);
                        color_row(
                            ui,
                            "Left underglow",
                            "LED 104–112",
                            &mut self.cfg.zones.left_underglow,
                        );
                        color_row(
                            ui,
                            "Right underglow",
                            "LED 114–122",
                            &mut self.cfg.zones.right_underglow,
                        );
                        color_row(
                            ui,
                            "Knob accent",
                            "LED 124",
                            &mut self.cfg.zones.knob_accent,
                        );
                    });

                // The knob LED doubles as a Caps Lock indicator, so this sits
                // directly under it rather than in its own section.
                ui.horizontal(|ui| {
                    let caps = &mut self.cfg.caps_lock_indicator;
                    ui.checkbox(&mut caps.enabled, "Indicate CAPS LOCK");
                    ui.add_enabled_ui(caps.enabled, |ui| {
                        ui.color_edit_button_srgb(&mut caps.color.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{}  ·  knob while on",
                                caps.color.to_hex()
                            ))
                            .small()
                            .weak()
                            .monospace(),
                        );
                    });
                });

                ui.add_space(12.0);
                ui.label(egui::RichText::new("REACTIVE EFFECT").small().weak());
                egui::Grid::new("reactive")
                    .num_columns(3)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        color_row(
                            ui,
                            "Press colour",
                            "on key down",
                            &mut self.cfg.reactive_effect.press_color,
                        );
                    });
                ui.horizontal(|ui| {
                    ui.label("Fade back");
                    ui.add(
                        egui::Slider::new(
                            &mut self.cfg.reactive_effect.fade_duration_sec,
                            0.05..=2.0,
                        )
                        .suffix(" s")
                        .fixed_decimals(2),
                    );
                });

                ui.add_space(12.0);
                ui.label(egui::RichText::new("BEHAVIOUR").small().weak());
                ui.checkbox(&mut self.cfg.start_hidden, "Start hidden in the tray");
                ui.label(
                    egui::RichText::new(
                        "Off: the window opens on launch and the X button hides it to the tray.",
                    )
                    .small()
                    .weak(),
                );

                ui.add_space(12.0);
                ui.label(egui::RichText::new("KEY MAPPING").small().weak());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} of {MAIN_KEY_COUNT} LEDs bound.",
                            self.keymap.bound_count()
                        ))
                        .small(),
                    );
                    if ui
                    .button("Learn keymap…")
                    .on_hover_text(
                        "Lights one LED at a time — press the key that lit up. Writes keymap.json.",
                    )
                    .clicked()
                {
                    let learn = Learn::new(&self.keymap);
                    learn.sync_probe();
                    self.learn = Some(learn);
                }
                    if ui
                        .button("Reset to built-in")
                        .on_hover_text(
                            "Restore the default 100% ANSI guess (does not write to disk).",
                        )
                        .clicked()
                    {
                        self.keymap = Keymap::default();
                        render::send(render::Msg::Keymap(Box::new(self.keymap.clone())));
                        self.notify("Key mapping reset to the built-in layout.");
                    }
                });

                ui.add_space(16.0);
                ui.separator();
                ui.horizontal(|ui| {
                    let dirty = self.cfg != self.saved_cfg;
                    if ui
                        .button("Save")
                        .on_hover_text(self.paths.config.display().to_string())
                        .clicked()
                    {
                        match self.cfg.save(&self.paths.config) {
                            Ok(()) => {
                                self.saved_cfg = self.cfg.clone();
                                self.notify("Saved config.json.");
                            }
                            Err(e) => self.notify(format!("Could not save: {e}")),
                        }
                    }
                    if ui.button("Reload from disk").clicked() {
                        let (cfg, warn) = Config::load(&self.paths.config);
                        self.cfg = cfg;
                        self.saved_cfg = self.cfg.clone();
                        self.warning = warn;
                        self.notify("Reloaded config.json.");
                    }
                    if dirty {
                        ui.label(egui::RichText::new("unsaved changes").small().weak());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("Shut down")
                            .on_hover_text("Quit completely and release the keyboard.")
                            .clicked()
                        {
                            tray::shutdown();
                        }
                    });
                });

                if let Some((_, text)) = &self.notice {
                    ui.label(egui::RichText::new(text).small());
                }
            });
    }

    fn status_ui(&mut self, ui: &mut egui::Ui) {
        let status = self.status.lock().map(|s| s.clone()).unwrap_or_default();
        let tint = if status.connected {
            egui::Color32::from_rgb(0x3D, 0xDC, 0x6B)
        } else if status.suspended {
            egui::Color32::from_rgb(0xC8, 0xA0, 0x30)
        } else {
            egui::Color32::from_rgb(0xD0, 0x4A, 0x4A)
        };
        let dot = "●";

        ui.horizontal(|ui| {
            ui.colored_label(tint, dot);
            if status.connected {
                ui.label(&status.device);
            } else {
                ui.label("Not connected");
            }
            if status.halted || (!status.connected && !status.suspended) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("Reconnect")
                        .on_hover_text("Try the keyboard again now.")
                        .clicked()
                    {
                        render::send(render::Msg::Reconnect);
                    }
                });
            }
        });
        if status.connected {
            // The counter only climbs when a frame actually differs from the
            // last one sent (plus a 1 Hz keepalive), so a near-static reading
            // while idle means the frame-skipping is doing its job.
            let handshake = if status.handshake.is_empty() {
                "unknown".to_string()
            } else {
                status.handshake.clone()
            };
            ui.label(
                egui::RichText::new(format!(
                    "protocol: {handshake}   ·   {} frames sent",
                    render::frames_sent()
                ))
                .small()
                .weak()
                .monospace(),
            );
        }
        if !status.message.is_empty() {
            ui.label(
                egui::RichText::new(&status.message)
                    .small()
                    .color(egui::Color32::from_rgb(0xE0, 0x9B, 0x3A)),
            );
        }
        if let Some(warning) = &self.warning {
            ui.label(
                egui::RichText::new(warning)
                    .small()
                    .color(egui::Color32::from_rgb(0xE0, 0x9B, 0x3A)),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Learn Keymap panel
    // -----------------------------------------------------------------------

    fn learn_ui(&mut self, ui: &mut egui::Ui) {
        let (idx, finished, captured_here, total_bound) = {
            let learn = self
                .learn
                .as_ref()
                .expect("learn_ui called without a session");
            (
                learn.idx,
                learn.finished(),
                learn.captured.get(learn.idx).copied().flatten(),
                learn.captured.iter().filter(|k| k.is_some()).count(),
            )
        };

        ui.heading("Learn keymap");
        ui.label(
            egui::RichText::new(
                "One LED on the keyboard is lit white. Press that key. \
                 Use Skip for positions with no key.",
            )
            .small()
            .weak(),
        );
        ui.add_space(8.0);

        if finished {
            ui.label(
                egui::RichText::new("All 104 positions visited.")
                    .heading()
                    .color(egui::Color32::from_rgb(0x3D, 0xDC, 0x6B)),
            );
        } else {
            ui.add(
                egui::ProgressBar::new(idx as f32 / MAIN_KEY_COUNT as f32)
                    .text(format!("LED {idx} of {MAIN_KEY_COUNT}")),
            );
            ui.add_space(8.0);
            ui.label(format!(
                "Expected on a standard ANSI board: {}",
                keymap::default_led_name(idx)
            ));
            match captured_here {
                Some(id) => ui.label(
                    egui::RichText::new(format!("currently bound to {}", keymap::key_name(id)))
                        .small(),
                ),
                None => ui.label(egui::RichText::new("currently unbound").small().weak()),
            };
        }

        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!("{total_bound} keys bound so far"))
                .small()
                .weak(),
        );
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if ui
                .add_enabled(idx > 0, egui::Button::new("← Back"))
                .clicked()
                && let Some(learn) = self.learn.as_mut()
            {
                learn.jump_to(learn.idx - 1);
            }
            if ui
                .add_enabled(!finished, egui::Button::new("Skip (no key here)"))
                .clicked()
                && let Some(learn) = self.learn.as_mut()
            {
                learn.captured[learn.idx] = None;
                learn.jump_to(learn.idx + 1);
            }
            if ui.button("Restart").clicked()
                && let Some(learn) = self.learn.as_mut()
            {
                learn.jump_to(0);
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .button("Save & apply")
                .on_hover_text(self.paths.keymap.display().to_string())
                .clicked()
            {
                let captured = self
                    .learn
                    .as_ref()
                    .map(|l| l.captured.clone())
                    .unwrap_or_default();
                let keymap = Keymap::from_slice(&captured);
                match keymap.save(&self.paths.keymap) {
                    Ok(()) => {
                        self.keymap = keymap;
                        render::send(render::Msg::Keymap(Box::new(self.keymap.clone())));
                        self.cancel_learn();
                        self.notify("Saved keymap.json.");
                    }
                    Err(e) => self.notify(format!("Could not save keymap.json: {e}")),
                }
            }
            if ui.button("Cancel").clicked() {
                self.cancel_learn();
            }
        });
    }
}

fn color_row(ui: &mut egui::Ui, label: &str, hint: &str, color: &mut HexColor) {
    ui.label(label);
    ui.color_edit_button_srgb(&mut color.0);
    ui.label(
        egui::RichText::new(format!("{}  ·  {hint}", color.to_hex()))
            .small()
            .weak()
            .monospace(),
    );
    ui.end_row();
}
