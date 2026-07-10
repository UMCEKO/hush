//! Safe-ish Rust bindings to NVIDIA's Maxine Audio Effects (AFX) denoiser.
//!
//! FFI over the small C API in `nvAudioEffects.h`. The user supplies the SDK +
//! model (downloaded from NGC under their own license); we never ship them.

use std::ffi::CString;
use std::os::raw::{c_char, c_float, c_int, c_uint, c_void};

use anyhow::{bail, Result};

pub mod engine;
pub mod ipc;
pub mod model;

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub use ipc::NotchParam;

/// Quick-add presets the UI exposes (50 Hz + 60 Hz mains harmonics).
pub const NOTCH_FREQS: [f32; 8] = [50.0, 60.0, 100.0, 120.0, 150.0, 180.0, 200.0, 240.0];
/// Spectrum bins shown to the UI (≈ 0..1000 Hz, where hum lives).
pub const SPECTRUM_BINS: usize = 86;
/// Hz per spectrum bin (48000 / 4096 FFT).
pub const SPECTRUM_BIN_HZ: f32 = 48_000.0 / 4096.0;

/// Live, lock-light control surface shared between the UI and the audio engine.
pub struct Controls {
    intensity: AtomicU32,                  // f32 bits, 0.0..1.0
    notches: Mutex<Vec<NotchParam>>,       // active parametric notches
    notch_gen: AtomicU64,                  // bumped on every notch change (cheap RT poll)
    pub spectrum: Mutex<Vec<f32>>,         // post-band magnitudes, 0..1 (denoised + notch)
    pub spectrum_in: Mutex<Vec<f32>>,      // pre-band magnitudes, 0..1 (denoised, same scale)
}

impl Controls {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            intensity: AtomicU32::new(1.0f32.to_bits()),
            notches: Mutex::new(Vec::new()),
            notch_gen: AtomicU64::new(0),
            spectrum: Mutex::new(vec![0.0; SPECTRUM_BINS]),
            spectrum_in: Mutex::new(vec![0.0; SPECTRUM_BINS]),
        })
    }
    pub fn set_intensity(&self, ratio: f32) {
        self.intensity.store(ratio.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
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
}

pub type Handle = *mut c_void;

#[link(name = "nv_audiofx")]
unsafe extern "C" {
    fn NvAFX_CreateEffect(code: *const c_char, effect: *mut Handle) -> c_int;
    fn NvAFX_DestroyEffect(effect: Handle) -> c_int;
    fn NvAFX_SetString(effect: Handle, name: *const c_char, val: *const c_char) -> c_int;
    fn NvAFX_SetU32(effect: Handle, name: *const c_char, val: c_uint) -> c_int;
    fn NvAFX_SetFloat(effect: Handle, name: *const c_char, val: c_float) -> c_int;
    fn NvAFX_GetU32(effect: Handle, name: *const c_char, val: *mut c_uint) -> c_int;
    fn NvAFX_Load(effect: Handle) -> c_int;
    fn NvAFX_Run(
        effect: Handle,
        input: *const *const c_float,
        output: *const *mut c_float,
        num_samples: c_uint,
        num_channels: c_uint,
    ) -> c_int;
}

fn ck(status: c_int, what: &str) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        bail!("{what}: NvAFX status {status}")
    }
}

/// A loaded Maxine denoiser effect bound to one GPU stream.
pub struct Denoiser {
    h: Handle,
    /// Samples per `process` call (the model's frame size, e.g. 480 @ 48 kHz).
    pub frame: usize,
}

// The handle is owned exclusively; safe to move to a worker thread.
unsafe impl Send for Denoiser {}

impl Denoiser {
    /// `version` = 1 (BNR) or 2 (the newer SEASR model — needs effect_version=2).
    pub fn new(model_path: &str, version: u32) -> Result<Self> {
        unsafe {
            let mut h: Handle = std::ptr::null_mut();
            ck(NvAFX_CreateEffect(c"denoiser".as_ptr(), &mut h), "CreateEffect")?;
            let cmodel = CString::new(model_path)?;
            ck(NvAFX_SetString(h, c"model_path".as_ptr(), cmodel.as_ptr()), "SetString(model_path)")?;
            ck(NvAFX_SetU32(h, c"sample_rate".as_ptr(), 48_000), "SetU32(sample_rate)")?;
            ck(NvAFX_SetU32(h, c"effect_version".as_ptr(), version), "SetU32(effect_version)")?;
            ck(NvAFX_SetU32(h, c"num_samples_per_frame".as_ptr(), 480), "SetU32(num_samples_per_frame)")?;
            ck(NvAFX_SetFloat(h, c"intensity_ratio".as_ptr(), 1.0), "SetFloat(intensity_ratio)")?;
            ck(NvAFX_Load(h), "Load")?;
            let mut ns: c_uint = 0;
            ck(NvAFX_GetU32(h, c"num_samples_per_frame".as_ptr(), &mut ns), "GetU32(num_samples_per_frame)")?;
            Ok(Self { h, frame: ns as usize })
        }
    }

    /// Live suppression strength: 0.0 = bypass (raw mic) .. 1.0 = full denoise.
    pub fn set_intensity(&mut self, ratio: f32) -> Result<()> {
        unsafe {
            ck(
                NvAFX_SetFloat(self.h, c"intensity_ratio".as_ptr(), ratio.clamp(0.0, 1.0)),
                "SetFloat(intensity_ratio)",
            )
        }
    }

    /// Denoise one `frame`-sized mono chunk in [-1, 1]. `input`/`output` len == `self.frame`.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<()> {
        assert_eq!(input.len(), self.frame);
        assert_eq!(output.len(), self.frame);
        unsafe {
            let in_ptrs: [*const c_float; 1] = [input.as_ptr()];
            let out_ptrs: [*mut c_float; 1] = [output.as_mut_ptr()];
            ck(
                NvAFX_Run(self.h, in_ptrs.as_ptr(), out_ptrs.as_ptr(), self.frame as c_uint, 1),
                "Run",
            )
        }
    }
}

impl Drop for Denoiser {
    fn drop(&mut self) {
        unsafe {
            NvAFX_DestroyEffect(self.h);
        }
    }
}
