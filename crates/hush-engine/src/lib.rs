//! GPU-linked half of HUSH: safe-ish Rust bindings to NVIDIA's Maxine Audio
//! Effects (AFX) denoiser, plus the real-time engine. Links `libnv_audiofx` +
//! `libcudart` (see build.rs) — so only the daemon (`hushd`) and dev tools depend
//! on this crate; the GUI links `hush-core` only.
//!
//! FFI over the small C API in `nvAudioEffects.h`. The user supplies the SDK +
//! model (downloaded at runtime under their own license); we never ship them.

use std::ffi::CString;
use std::os::raw::{c_char, c_float, c_int, c_uint, c_void};

use anyhow::{Result, bail};

pub mod engine;

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
            ck(
                NvAFX_CreateEffect(c"denoiser".as_ptr(), &mut h),
                "CreateEffect",
            )?;
            let cmodel = CString::new(model_path)?;
            ck(
                NvAFX_SetString(h, c"model_path".as_ptr(), cmodel.as_ptr()),
                "SetString(model_path)",
            )?;
            ck(
                NvAFX_SetU32(h, c"sample_rate".as_ptr(), 48_000),
                "SetU32(sample_rate)",
            )?;
            ck(
                NvAFX_SetU32(h, c"effect_version".as_ptr(), version),
                "SetU32(effect_version)",
            )?;
            ck(
                NvAFX_SetU32(h, c"num_samples_per_frame".as_ptr(), 480),
                "SetU32(num_samples_per_frame)",
            )?;
            ck(
                NvAFX_SetFloat(h, c"intensity_ratio".as_ptr(), 1.0),
                "SetFloat(intensity_ratio)",
            )?;
            ck(NvAFX_Load(h), "Load")?;
            let mut ns: c_uint = 0;
            ck(
                NvAFX_GetU32(h, c"num_samples_per_frame".as_ptr(), &mut ns),
                "GetU32(num_samples_per_frame)",
            )?;
            Ok(Self {
                h,
                frame: ns as usize,
            })
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
                NvAFX_Run(
                    self.h,
                    in_ptrs.as_ptr(),
                    out_ptrs.as_ptr(),
                    self.frame as c_uint,
                    1,
                ),
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
