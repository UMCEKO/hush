//! Wire protocol between the HUSH GUI (client) and the `hushd` daemon (server).
//!
//! Transport is newline-delimited JSON over a Unix domain socket: the GUI sends
//! [`ClientMsg`] control changes, the daemon streams [`StateFrame`] snapshots back
//! (~30 Hz) for the spectrum/level display. Either side can come and go freely.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One parametric EQ cut: centre frequency (Hz), Q (higher = narrower), and the
/// cut depth in dB (<= 0; very negative ≈ a full notch). `enabled` lets the GUI
/// keep a band in the rack while muting its effect on the audio.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct NotchParam {
    pub freq: f32,
    pub q: f32,
    pub gain: f32,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// A control change the GUI sends to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Suppression strength, 0.0..=1.0 (0 = bypass / raw mic).
    Intensity { value: f32 },
    /// Replace the full set of active hum notches.
    SetNotches { notches: Vec<NotchParam> },
    /// Ask the daemon to clean up and exit 0 (so a non-systemd sibling daemon can
    /// be respawned by the GUI — used when `systemctl restart` isn't available).
    Shutdown,
}

/// A snapshot the daemon streams to attached GUIs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateFrame {
    pub intensity: f32,
    pub notches: Vec<NotchParam>,
    pub model_version: u32,
    pub spectrum: Vec<f32>,    // adjusted output (post denoise+notch)
    #[serde(default)]
    pub spectrum_in: Vec<f32>, // original mic, same scale as `spectrum`
    /// No model resolved — the GUI must download one before the engine can run.
    #[serde(default)]
    pub model_missing: bool,
    /// Engine/denoiser initialisation failed (e.g. incompatible model).
    #[serde(default)]
    pub engine_error: Option<String>,
    /// GPU the daemon is running Maxine on (for the settings display).
    #[serde(default)]
    pub gpu_name: Option<String>,
}

/// Control-socket path: `$XDG_RUNTIME_DIR/hush.sock`, falling back to `/tmp`.
pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("hush.sock"),
        None => PathBuf::from("/tmp/hush.sock"),
    }
}
