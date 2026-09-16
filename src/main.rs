// No console window behind the GUI when the exe is double-clicked.
#![windows_subsystem = "windows"]

mod config;
mod conflicts;
mod debug;
mod gui;
mod icon;
mod input;
mod instance;
mod keymap;
mod probe;
mod protocol;
mod render;
mod tray;

use std::sync::{Arc, Mutex};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--probe") {
        probe::run();
        return;
    }
    if args.iter().any(|a| a == "--debug") {
        let _ = debug::run();
        return;
    }

    // Two copies would fight over the exclusive Raw HID handle. Hand our intent
    // to the copy that is already running and bow out.
    let Some(_instance) = instance::acquire() else {
        instance::signal_existing();
        return;
    };

    input::init_clock();
    // Read the initial Caps Lock state from a thread that has just been given a
    // message queue, which is the one moment GetKeyState is trustworthy for a
    // toggle key — see input.rs. From here on the hook tracks it.
    input::seed_caps_lock();

    let paths = config::Paths::discover();
    let (cfg, cfg_warning) = config::Config::load(&paths.config);
    let (km, km_warning) = keymap::Keymap::load(&paths.keymap);
    // Only one warning line fits comfortably; the config one matters more.
    let warning = cfg_warning.or(km_warning);

    let status = Arc::new(Mutex::new(render::Status::default()));

    // Low-level keyboard hook + power-broadcast listener.
    input::spawn();
    // Owns the USB handle and the 60 FPS loop.
    render::spawn(cfg.clone(), km.clone(), Arc::clone(&status));

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(gui::WINDOW_TITLE)
        .with_inner_size([460.0, 640.0])
        .with_min_inner_size([420.0, 360.0])
        .with_icon(egui::IconData {
            rgba: icon::rgba(),
            width: icon::SIZE,
            height: icon::SIZE,
        });
    if cfg.start_hidden {
        viewport = viewport.with_visible(false);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let result = eframe::run_native(
        gui::WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            // Published so the tray thread and the hidden power window can wake
            // the event loop while the GUI is hidden.
            tray::set_ui_ctx(cc.egui_ctx.clone());
            // The tray runs its own message pump, so it keeps working while the
            // window is hidden and the winit loop is idle.
            tray::spawn();
            Ok(Box::new(gui::App::new(cc, paths, cfg, km, warning, status)))
        }),
    );

    // Reaching here means the event loop really did exit (not a hide-to-tray).
    if result.is_err() {
        input::uninstall_hook();
        std::process::exit(1);
    }
    render::send(render::Msg::Shutdown);
    input::uninstall_hook();
}
