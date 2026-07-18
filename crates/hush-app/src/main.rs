//! HUSH — noise-suppression virtual mic. Control-deck UI over the live Maxine
//! engine, with a first-run license screen. Webview default; `NV_MAXINE_BLITZ=1`
//! uses the native Blitz/wgpu renderer (build with `--features blitz`).
//!
//! This file is the entry point only. The GUI lives in the sibling modules:
//! `components/` (pages + shell), `state` (cross-thread mirrors), `ipc` (daemon
//! sync), `service` (systemd unit), `window`/`tray` (surfacing), `style` (CSS).

mod components;
mod ipc;
mod service;
mod state;
mod style;
mod tray;
mod window;

use components::App;
use hush_core::Controls;
use state::{CLOSE_TO_TRAY, CONTROLS, load_close_to_tray};

fn main() {
    // Single instance: poke any running GUI to surface, then bail if one holds the lock.
    window::notify_show_if_running();
    let _lock = match window::acquire_single_instance() {
        Some(f) => f,
        None => {
            eprintln!("HUSH is already running — raising the existing window.");
            return;
        }
    };
    window::spawn_show_listener();
    CLOSE_TO_TRAY.store(load_close_to_tray(), std::sync::atomic::Ordering::Relaxed);

    // `CONTROLS` is now a local *mirror* of the daemon's state, not the engine's:
    // the UI reads/writes it exactly as before, and a background thread syncs it to
    // `hushd` over the control socket. Closing the GUI just drops that socket.
    let controls = Controls::new();
    let _ = CONTROLS.set(controls.clone());
    ipc::spawn_ipc_sync(controls);

    #[cfg(feature = "blitz")]
    if state::blitz_on() {
        let attrs = dioxus_native::WindowAttributes::default()
            .with_title("HUSH")
            .with_decorations(false)
            .with_surface_size(dioxus_native::LogicalSize::new(480.0, 720.0));
        dioxus_native::launch_cfg(
            App,
            vec![],
            vec![Box::new(
                dioxus_native::Config::new().with_window_attributes(attrs),
            )],
        );
        return;
    }

    use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
    let win = WindowBuilder::new()
        .with_title("HUSH")
        .with_inner_size(LogicalSize::new(480.0, 720.0))
        .with_decorations(false)
        .with_resizable(false)
        .with_window_icon(tray::window_icon());
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(win))
        .launch(App);
}
