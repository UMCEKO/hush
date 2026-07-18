//! Cross-thread GUI state: mirrors of the daemon's `StateFrame`, live download
//! progress, persisted on-disk flags, and the `Ctl` context shared by pages.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use dioxus::prelude::*;
use hush_core::ipc::MicInfo;
use hush_core::{Controls, NotchParam};

pub(crate) static CONTROLS: OnceLock<Arc<Controls>> = OnceLock::new();
/// True while the GUI is attached to a live `hushd`.
pub(crate) static CONNECTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// When set, the ✕ button hides the window to the tray instead of quitting.
pub(crate) static CLOSE_TO_TRAY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// ---- setup / model-provisioning state, mirrored from the daemon's StateFrame ----
/// The daemon has no usable model — route the GUI to the setup/download page.
pub(crate) static MODEL_MISSING: AtomicBool = AtomicBool::new(false);
/// Engine init failed (bad model, no GPU, …); shown on the setup page.
pub(crate) static ENGINE_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// GPU the daemon reports running on (for the settings row).
pub(crate) static GPU_NAME: Mutex<Option<String>> = Mutex::new(None);
/// Capture devices + active pick, mirrored from the daemon's `StateFrame`.
pub(crate) static MICS: Mutex<Vec<MicInfo>> = Mutex::new(Vec::new());
pub(crate) static ACTIVE_MIC: Mutex<Option<String>> = Mutex::new(None);
/// The user picked a mic this session — push it on (re)connect instead of
/// adopting the daemon's persisted choice.
pub(crate) static MIC_TOUCHED: AtomicBool = AtomicBool::new(false);
/// Set by the settings GPU picker to force the setup page (new arch needs a model).
pub(crate) static SETUP_REQUESTED: AtomicBool = AtomicBool::new(false);

// ---- live model-download progress (written by the download thread) ----
pub(crate) const DL_IDLE: u8 = 0;
pub(crate) const DL_RUNNING: u8 = 1;
pub(crate) const DL_DONE: u8 = 2;
pub(crate) const DL_FAILED: u8 = 3;
pub(crate) static DL_PHASE: AtomicU8 = AtomicU8::new(DL_IDLE);
pub(crate) static DL_GOT: AtomicU64 = AtomicU64::new(0);
pub(crate) static DL_TOTAL: AtomicU64 = AtomicU64::new(0);
pub(crate) static DL_ERROR: Mutex<Option<String>> = Mutex::new(None);
// Which artifact is in flight, for the progress label.
pub(crate) const STAGE_SDK: u8 = 0;
pub(crate) const STAGE_MODEL: u8 = 1;
pub(crate) const STAGE_UNPACK: u8 = 2;
pub(crate) static DL_STAGE: AtomicU8 = AtomicU8::new(STAGE_SDK);

fn ctray_path() -> PathBuf {
    hush_core::data_home().join("hush/close_to_tray")
}
pub(crate) fn load_close_to_tray() -> bool {
    ctray_path().exists()
}
pub(crate) fn persist_close_to_tray(on: bool) {
    let p = ctray_path();
    if on {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, "1");
    } else {
        let _ = std::fs::remove_file(p);
    }
}

#[allow(dead_code)]
pub(crate) fn blitz_on() -> bool {
    std::env::var_os("NV_MAXINE_BLITZ").is_some_and(|v| v == "1")
}

/// Are we actually running the Blitz renderer (vs the default webview)?
pub(crate) fn running_blitz() -> bool {
    cfg!(feature = "blitz") && blitz_on()
}

fn flag_path() -> PathBuf {
    hush_core::data_home().join("hush/eula_accepted")
}
pub(crate) fn eula_accepted() -> bool {
    flag_path().exists()
}
pub(crate) fn persist_accept() {
    let p = flag_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, "1");
}

/// Shared live control state, provided via context to every page.
#[derive(Clone, Copy)]
pub(crate) struct Ctl {
    pub(crate) power: Signal<bool>,
    pub(crate) level: Signal<u32>,
    pub(crate) notches: Signal<Vec<NotchParam>>,
    pub(crate) spectrum: Signal<Vec<f32>>,    // adjusted output
    pub(crate) spectrum_in: Signal<Vec<f32>>, // original mic
}

/// Whether to route to the setup page: the SDK runtime isn't provisioned yet (the
/// daemon can't even exec without it), the daemon reports no model, an engine
/// error, or the settings GPU picker asked for it.
pub(crate) fn setup_needed() -> bool {
    !hush_core::sdk::sdk_ready()
        || MODEL_MISSING.load(Ordering::Relaxed)
        || SETUP_REQUESTED.load(Ordering::Relaxed)
        || ENGINE_ERROR
            .lock()
            .ok()
            .map(|g| g.is_some())
            .unwrap_or(false)
}
