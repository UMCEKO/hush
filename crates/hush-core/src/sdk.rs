//! Maxine SDK runtime provisioning. HUSH ships no NVIDIA bits; the runtime libs
//! (libnv_audiofx + the denoiser feature lib + the CUDA/TensorRT stack it dlopens)
//! are downloaded once from the CDN and unpacked into the XDG data dir, exactly
//! like the per-GPU models. `hushd` links these at exec via `LD_LIBRARY_PATH`
//! (built from `ld_library_path()`); the GUI never links them.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::data_home;
use crate::model::{cdn_base, hex};

/// SDK release we target (Maxine AFX). Bump when a new SDK is mirrored.
pub const SDK_VERSION: &str = "2.1.0";

/// One CDN artifact: its object key (under the version dir), sha256, byte size.
pub struct SdkArtifact {
    pub key: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// Full runtime tarball (nvafx + denoiser feature + CUDA/TensorRT). Filled from
/// `scripts/make-sdk-tarballs.sh` output.
pub const RUNTIME: SdkArtifact = SdkArtifact {
    key: "afx-runtime-x86_64.tar.zst",
    sha256: "aa9ea4002d738a0ee82ad493638525f582f1f6ff589b9b9a52e203868e5837e7",
    size: 1_081_966_146,
};

/// Minimal link tarball (libnv_audiofx + libcudart) — used by CI/AUR/Nix builds,
/// not the runtime download.
pub const LINK: SdkArtifact = SdkArtifact {
    key: "afx-link-x86_64.tar.zst",
    sha256: "eb381134dcfae78a35b85f06c6d2c87103efbb7e736e0dcbcd3daa4fc359eba5",
    size: 336_165,
};

/// Legacy hand-installed SDK location (the author's dev box).
fn legacy_sdk() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("maxine-dl/sdk/Audio_Effects_SDK")
}

/// Where a CDN-downloaded SDK is unpacked.
fn installed_sdk() -> PathBuf {
    data_home().join(format!("hush/sdk/{SDK_VERSION}"))
}

fn has_runtime(root: &Path) -> bool {
    root.join("nvafx/lib/libnv_audiofx.so.2").exists()
}

/// Resolve a usable SDK root, or `None` if the runtime still needs downloading.
/// Order: `$NVAFX_SDK` (dev override) > legacy `~/maxine-dl` > verified XDG install.
pub fn sdk_root() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NVAFX_SDK") {
        let p = PathBuf::from(p);
        if has_runtime(&p) {
            return Some(p);
        }
    }
    let legacy = legacy_sdk();
    if has_runtime(&legacy) {
        return Some(legacy);
    }
    let installed = installed_sdk();
    // The `.verified` marker means the tarball was sha256-checked before extraction,
    // so we trust the unpacked tree without re-hashing 2.3 GB on every start.
    if installed.join(".verified").exists() && has_runtime(&installed) {
        return Some(installed);
    }
    None
}

/// The three lib dirs the loader needs, in priority order.
pub fn lib_dirs(root: &Path) -> Vec<PathBuf> {
    vec![
        root.join("nvafx/lib"),
        root.join("external/cuda/lib"),
        root.join("features/denoiser/lib"),
    ]
}

/// `LD_LIBRARY_PATH` value for launching `hushd`: the SDK lib dirs, the host
/// driver dir (libcuda, harmless if absent), then any inherited value.
pub fn ld_library_path() -> Option<String> {
    let root = sdk_root()?;
    let mut parts: Vec<String> = lib_dirs(&root)
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    parts.push("/run/opengl-driver/lib".to_string());
    if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH")
        && !existing.is_empty()
    {
        parts.push(existing.to_string_lossy().into_owned());
    }
    Some(parts.join(":"))
}

/// True once a usable SDK is present (GUI setup gate).
pub fn sdk_ready() -> bool {
    sdk_root().is_some()
}

/// Download + verify + unpack the runtime SDK tarball. Blocking — call on a
/// dedicated `std::thread` (reqwest::blocking panics inside a tokio context).
/// `progress(done, total)` fires during the download; extraction is a final step.
pub fn download_sdk(progress: &(dyn Fn(u64, u64) + Send + Sync)) -> Result<PathBuf> {
    let dir = data_home().join("hush/sdk");
    std::fs::create_dir_all(&dir).context("create sdk dir")?;
    let tmp = dir.join(format!("{}.part", RUNTIME.key));
    let url = format!("{}/sdk/{SDK_VERSION}/{}", cdn_base(), RUNTIME.key);

    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .context("build http client")?;
    let mut resp = client
        .get(&url)
        .send()
        .with_context(|| format!("request {url}"))?
        .error_for_status()
        .with_context(|| format!("SDK URL returned an error: {url}"))?;
    let total = resp.content_length().unwrap_or(RUNTIME.size);

    let mut out = std::fs::File::create(&tmp).context("create temp file")?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut done: u64 = 0;
    progress(0, total);
    loop {
        let n = resp.read(&mut buf).context("read sdk stream")?;
        if n == 0 {
            break;
        }
        use std::io::Write;
        out.write_all(&buf[..n]).context("write sdk")?;
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    drop(out);

    let got = hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(RUNTIME.sha256) || (RUNTIME.size != 0 && done != RUNTIME.size) {
        let _ = std::fs::remove_file(&tmp);
        bail!(
            "SDK integrity check failed — expected {}, got {got}",
            RUNTIME.sha256
        );
    }

    // Extract into a staging dir, then atomically swap into place.
    let staging = dir.join(format!("{SDK_VERSION}.partial"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).context("create staging dir")?;
    extract_zst_tar(&tmp, &staging).context("unpack SDK tarball")?;
    let _ = std::fs::remove_file(&tmp);

    let final_dir = installed_sdk();
    let _ = std::fs::remove_dir_all(&final_dir);
    std::fs::rename(&staging, &final_dir).context("install SDK")?;
    std::fs::write(final_dir.join(".verified"), RUNTIME.sha256).context("write marker")?;
    Ok(final_dir)
}

fn extract_zst_tar(tarball: &Path, dest: &Path) -> Result<()> {
    let f = std::fs::File::open(tarball)?;
    let dec = zstd::stream::read::Decoder::new(f)?;
    let mut ar = tar::Archive::new(dec);
    ar.set_preserve_permissions(true);
    ar.set_overwrite(true);
    ar.unpack(dest)?;
    Ok(())
}

/// Verify an already-installed SDK's marker matches a known artifact (diagnostic).
pub fn installed_matches(art: &SdkArtifact) -> bool {
    let marker = installed_sdk().join(".verified");
    std::fs::read_to_string(marker)
        .map(|s| s.trim().eq_ignore_ascii_case(art.sha256))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end SDK download → verify → unpack → resolve. Gated on
    /// `HUSH_TEST_SDK=1`, with a fake `HOME`/`XDG_DATA_HOME` and `HUSH_MODEL_BASE`
    /// pointing at a mirror serving `sdk/2.1.0/afx-runtime-x86_64.tar.zst`.
    #[test]
    fn download_unpack_resolve() {
        if std::env::var("HUSH_TEST_SDK").as_deref() != Ok("1") {
            return;
        }
        // no NVAFX_SDK / legacy dir in the fake env → starts unresolved
        assert!(sdk_root().is_none(), "expected no SDK before download");
        let root = download_sdk(&|_, _| {}).expect("sdk download+unpack");
        assert!(
            has_runtime(&root),
            "unpacked tree must have libnv_audiofx.so.2"
        );
        assert!(root.join(".verified").exists(), "marker written");
        assert_eq!(
            sdk_root().as_deref(),
            Some(root.as_path()),
            "resolves to install"
        );
        assert!(sdk_ready());
        // ld_library_path includes the three lib dirs
        let ld = ld_library_path().unwrap();
        assert!(ld.contains("nvafx/lib") && ld.contains("external/cuda/lib"));
    }
}
