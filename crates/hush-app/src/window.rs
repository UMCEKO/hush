//! Window surfacing + single-instance plumbing: GTK-level show/hide reachable
//! from any thread, the runtime lockfile, and the "show" poke socket.

use std::path::PathBuf;

thread_local! {
    /// GTK handle of the main window, stashed on the GTK main thread at startup
    /// so `set_window_visible_from_any_thread` closures can reach it.
    pub(crate) static GTK_WINDOW: std::cell::RefCell<Option<gtk::ApplicationWindow>> =
        const { std::cell::RefCell::new(None) };
}

/// Show/hide the window at the GTK layer, callable from any thread (tray, GUI
/// socket listener). This must NOT go through dioxus state or futures: while
/// the window is hidden WebKit stops flushing edits, which stalls the vdom
/// polling loop — a component future waiting for a "show" ping would never be
/// polled again, stranding the window in the tray forever.
pub(crate) fn set_window_visible_from_any_thread(visible: bool) {
    glib::MainContext::default().invoke(move || {
        use gtk::prelude::*;
        GTK_WINDOW.with(|w| {
            if let Some(w) = &*w.borrow() {
                eprintln!("hush: gtk set visible={visible} (was {})", w.is_visible());
                if visible {
                    w.show_all();
                    w.deiconify();
                    w.present();
                } else {
                    w.hide();
                }
            }
        });
    });
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Hold an exclusive `flock` on a runtime lockfile for the process lifetime, so a
/// second GUI launch bails out. The lock releases automatically on exit (even crash).
pub(crate) fn acquire_single_instance() -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false) // it's a lock file — content is irrelevant, never truncate
        .write(true)
        .open(runtime_dir().join("hush-gui.lock"))
        .ok()?;
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (rc == 0).then_some(f)
}

fn gui_sock_path() -> PathBuf {
    runtime_dir().join("hush-gui.sock")
}

/// Best-effort: ask an already-running GUI to surface its window. Works on any DE,
/// so "close to tray" never strands the window even without a tray host.
pub(crate) fn notify_show_if_running() {
    use std::io::Write;
    if let Ok(mut s) = std::os::unix::net::UnixStream::connect(gui_sock_path()) {
        let _ = s.write_all(b"show\n");
    }
}

/// Listen for "show" pokes from later launches; flip `SHOW_REQUESTED` for the UI.
pub(crate) fn spawn_show_listener() {
    let path = gui_sock_path();
    let _ = std::fs::remove_file(&path);
    if let Ok(listener) = std::os::unix::net::UnixListener::bind(&path) {
        std::thread::spawn(move || {
            use std::io::Read;
            for mut c in listener.incoming().flatten() {
                let mut buf = [0u8; 8];
                let n = c.read(&mut buf).unwrap_or(0);
                // "hide" is a scripting aid; anything else surfaces the window.
                set_window_visible_from_any_thread(!buf[..n].starts_with(b"hide"));
            }
        });
    }
}
