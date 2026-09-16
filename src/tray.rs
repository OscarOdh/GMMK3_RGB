//! System tray, on its own thread with its own message pump.
//!
//! It has to be separate: showing the context menu blocks the pump until the
//! menu closes, and the input thread's pump must never block or the low-level
//! keyboard hook stalls typing for the whole system.

use crate::icon;
use crate::input;
use crate::render;
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

const ID_SHOW: &str = "show";
const ID_QUIT: &str = "quit";

/// Set when the user asks for the window back. The GUI clears it.
pub static SHOW_REQUEST: AtomicBool = AtomicBool::new(false);
static UI_CTX: OnceLock<egui::Context> = OnceLock::new();

/// Publish the egui context so any thread can wake the event loop. Call once,
/// from the app creator.
pub fn set_ui_ctx(ctx: egui::Context) {
    let _ = UI_CTX.set(ctx);
}

/// Ask the GUI to come back. Safe from any thread — the tray handlers and the
/// hidden power window (for a second launch of the exe) both use it.
pub fn request_show() {
    SHOW_REQUEST.store(true, Ordering::SeqCst);
    // This wakes the event loop even while the window is hidden, which is what
    // gets `App::logic` called so it can make the viewport visible again.
    if let Some(ctx) = UI_CTX.get() {
        ctx.request_repaint();
    }
}

pub fn spawn() {
    let _ = std::thread::Builder::new()
        .name("tray".to_string())
        .spawn(run);
}

fn run() {
    let icon = Icon::from_rgba(icon::rgba(), icon::SIZE, icon::SIZE).ok();

    let show = MenuItem::with_id(ID_SHOW, "Show window", true, None);
    let quit = MenuItem::with_id(ID_QUIT, "Shut down GMMK3 RGB", true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[&show, &PredefinedMenuItem::separator(), &quit]);

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("GMMK3 RGB Controller")
        .with_menu_on_left_click(false);
    if let Some(icon) = icon {
        builder = builder.with_icon(icon);
    }

    // Kept alive for the life of the thread; dropping it removes the icon.
    let _tray = match builder.build() {
        Ok(tray) => tray,
        Err(_) => return,
    };

    MenuEvent::set_event_handler(Some(|event: MenuEvent| match event.id.0.as_str() {
        ID_SHOW => request_show(),
        ID_QUIT => shutdown(),
        _ => {}
    }));
    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            request_show();
        }
    }));

    let mut msg = MSG::default();
    loop {
        let got = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
        if got <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Leave the board on a clean static frame, then go.
pub fn shutdown() {
    render::send(render::Msg::Shutdown);
    let deadline = Instant::now() + Duration::from_millis(600);
    while !render::is_done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    input::uninstall_hook();
    std::process::exit(0);
}
