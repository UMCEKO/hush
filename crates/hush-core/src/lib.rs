//! Core, GPU-free half of HUSH: the control surface shared between the GUI and
//! the engine, the IPC wire types, and model/SDK provisioning. Contains NO NVIDIA
//! FFI, so the GUI (which links only this) builds and runs on any machine — the
//! Maxine SDK is provisioned at runtime before the daemon starts.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub mod ipc;
pub mod model;
pub mod sdk;

pub use ipc::NotchParam;

/// Quick-add presets the UI exposes (50 Hz + 60 Hz mains harmonics).
pub const NOTCH_FREQS: [f32; 8] = [50.0, 60.0, 100.0, 120.0, 150.0, 180.0, 200.0, 240.0];
/// Spectrum bins shown to the UI (≈ 0..1000 Hz, where hum lives).
pub const SPECTRUM_BINS: usize = 86;
/// Hz per spectrum bin (48000 / 4096 FFT).
pub const SPECTRUM_BIN_HZ: f32 = 48_000.0 / 4096.0;

/// `$XDG_DATA_HOME`, falling back to `~/.local/share`. Under Flatpak this is the
/// per-app `~/.var/app/<id>/data`, so all persisted state stays inside the sandbox.
pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
}

/// Running inside a Flatpak sandbox? (Host tools like `nvidia-smi` aren't present
/// and must be reached with `flatpak-spawn --host`.)
pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// A `Command` for a HOST program: transparently `flatpak-spawn --host <prog>`
/// under Flatpak, or a plain command otherwise.
pub fn host_command(program: &str) -> std::process::Command {
    if in_flatpak() {
        let mut c = std::process::Command::new("flatpak-spawn");
        c.arg("--host").arg(program);
        c
    } else {
        std::process::Command::new(program)
    }
}

/// Persisted mic preference: the source's node name in `<data>/hush/mic`
/// (file absent = follow the system default).
pub fn mic_pref_path() -> PathBuf {
    data_home().join("hush/mic")
}
pub fn load_mic_pref() -> Option<String> {
    std::fs::read_to_string(mic_pref_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
pub fn save_mic_pref(name: Option<&str>) {
    let path = mic_pref_path();
    match name {
        Some(n) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, n);
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Live, lock-light control surface shared between the UI and the audio engine.
pub struct Controls {
    intensity: AtomicU32,             // f32 bits, 0.0..1.0
    notches: Mutex<Vec<NotchParam>>,  // active parametric notches
    notch_gen: AtomicU64,             // bumped on every notch change (cheap RT poll)
    mic: Mutex<Option<String>>,       // selected capture source (None = default)
    mic_gen: AtomicU64,               // bumped on every selection change
    pub spectrum: Mutex<Vec<f32>>,    // post-band magnitudes, 0..1 (denoised + notch)
    pub spectrum_in: Mutex<Vec<f32>>, // pre-band magnitudes, 0..1 (denoised, same scale)
}

impl Controls {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            intensity: AtomicU32::new(1.0f32.to_bits()),
            notches: Mutex::new(Vec::new()),
            notch_gen: AtomicU64::new(0),
            mic: Mutex::new(None),
            mic_gen: AtomicU64::new(0),
            spectrum: Mutex::new(vec![0.0; SPECTRUM_BINS]),
            spectrum_in: Mutex::new(vec![0.0; SPECTRUM_BINS]),
        })
    }
    pub fn set_intensity(&self, ratio: f32) {
        self.intensity
            .store(ratio.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn intensity(&self) -> f32 {
        f32::from_bits(self.intensity.load(Ordering::Relaxed))
    }
    /// Replace the active notch set (UI/daemon side).
    pub fn set_notches(&self, notches: Vec<NotchParam>) {
        if let Ok(mut g) = self.notches.lock() {
            *g = notches;
        }
        self.notch_gen.fetch_add(1, Ordering::Relaxed);
    }
    /// Generation counter — compare cheaply on the audio thread; only lock on change.
    pub fn notch_gen(&self) -> u64 {
        self.notch_gen.load(Ordering::Relaxed)
    }
    pub fn notches_snapshot(&self) -> Vec<NotchParam> {
        self.notches.lock().map(|g| g.clone()).unwrap_or_default()
    }
    /// Select the capture source (`None` = system default).
    pub fn set_mic(&self, name: Option<String>) {
        if let Ok(mut g) = self.mic.lock() {
            *g = name;
        }
        self.mic_gen.fetch_add(1, Ordering::Relaxed);
    }
    pub fn mic(&self) -> Option<String> {
        self.mic.lock().ok().and_then(|g| g.clone())
    }
    /// Generation counter — lets the GUI delta-send only real selection changes.
    pub fn mic_gen(&self) -> u64 {
        self.mic_gen.load(Ordering::Relaxed)
    }
}
