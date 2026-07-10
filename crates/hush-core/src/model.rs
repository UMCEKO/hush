//! Model provisioning: enumerate NVIDIA GPUs, resolve the matching Maxine denoiser
//! `.trtpkg` (env override > local SDK > integrity-verified cache), and — driven
//! explicitly by the GUI, never the daemon — download + sha256-verify it.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// Base URL of the public model mirror (Cloudflare R2 `hush` bucket via a custom
/// domain). Override with `HUSH_MODEL_BASE`. Objects live at
/// `<base>/models/sm_<cc>/denoiser[_v2]_48k.trtpkg`.
const MODEL_BASE: &str = "https://cdn.hush.umceko.com";

/// Compute capabilities we host models for (Maxine AFX 2.1.0). Pre-Turing GPUs
/// (< sm_75) are unsupported by the SDK itself.
pub const SUPPORTED_SM: [u32; 7] = [75, 80, 86, 89, 90, 100, 120];

/// One mirrored model: its arch, denoiser version, expected sha256 and byte size.
pub struct ModelEntry {
    pub sm: u32,
    pub version: u32,
    pub sha256: &'static str,
    pub size: u64,
}

/// Integrity manifest for every mirrored `.trtpkg` (sha256 of the R2-served bytes).
pub const MANIFEST: [ModelEntry; 14] = [
    ModelEntry {
        sm: 75,
        version: 1,
        sha256: "5c27cffcdafcd13d992b94f11a74eb460b36a2eedd2387cdbd2b2a7b9aade3aa",
        size: 35_700_064,
    },
    ModelEntry {
        sm: 75,
        version: 2,
        sha256: "7d6d45eadabdb5a2d2392c33e46923799daff00e6e117c7be682dd52935e74ba",
        size: 144_193_184,
    },
    ModelEntry {
        sm: 80,
        version: 1,
        sha256: "22505838467b209db641ba8e3ec471676fe4b52f3097ab05b268519695891be1",
        size: 42_147_536,
    },
    ModelEntry {
        sm: 80,
        version: 2,
        sha256: "1b1d4b52e10dc016add2cf13d3f773034b965c387b4a54ec53de7a6dbb8c002f",
        size: 344_588_448,
    },
    ModelEntry {
        sm: 86,
        version: 1,
        sha256: "aebb929a64ce9a1445d70a0421a27b22f3766273f522e6fb78ed1aa977bc7b37",
        size: 37_063_024,
    },
    ModelEntry {
        sm: 86,
        version: 2,
        sha256: "1fd2be8e16200e2f02bbf01f54e74e9d7ddabb54492b351810450f7a4e03315f",
        size: 126_368_160,
    },
    ModelEntry {
        sm: 89,
        version: 1,
        sha256: "1fae019ccd115f4790a77a51be26b396c18be86c49a7961f35fcfed43fdd8998",
        size: 40_689_296,
    },
    ModelEntry {
        sm: 89,
        version: 2,
        sha256: "99939fbef29bf1dfa00259ed4e04884541656e6821eaa00e91d8716340d8c0a6",
        size: 138_570_352,
    },
    ModelEntry {
        sm: 90,
        version: 1,
        sha256: "308d7fbe427b9298a7b7c99849d2b8a5ab1969a5e71e6328679eb5821cd23a9a",
        size: 38_731_616,
    },
    ModelEntry {
        sm: 90,
        version: 2,
        sha256: "7e7e81bb86331d41635c9295afc524a1a6e1a406cf4009105348d68d865ab8c7",
        size: 118_882_944,
    },
    ModelEntry {
        sm: 100,
        version: 1,
        sha256: "f81c23da2f0111fd471ceef114fde0db745b750b6cbcb56183b371576195bf60",
        size: 51_323_872,
    },
    ModelEntry {
        sm: 100,
        version: 2,
        sha256: "c1468ea6703942270f90f913e595cec3f4355f8da3ab8fe66a7ebf4bdbb127a1",
        size: 118_019_344,
    },
    ModelEntry {
        sm: 120,
        version: 1,
        sha256: "6bedf4eaaeeb01abe5d160670524d2cfb50aa869c5bea75850a55720b4bd710a",
        size: 61_749_312,
    },
    ModelEntry {
        sm: 120,
        version: 2,
        sha256: "b5c3554abcda0670e6c3ae4f55d85c5af885efb5cee32734c3d0bdbce88771fb",
        size: 124_105_408,
    },
];

pub fn manifest_entry(version: u32, sm: u32) -> Option<&'static ModelEntry> {
    MANIFEST.iter().find(|e| e.sm == sm && e.version == version)
}

/// Base URL of the CDN mirror (models + SDK), overridable for tests.
pub(crate) fn cdn_base() -> String {
    std::env::var("HUSH_MODEL_BASE").unwrap_or_else(|_| MODEL_BASE.to_string())
}

/// Stable model filename for a denoiser version (2 = v2 48k, else v1 48k).
pub fn model_file(version: u32) -> &'static str {
    if version == 2 {
        "denoiser_v2_48k.trtpkg"
    } else {
        "denoiser_48k.trtpkg"
    }
}

// ---------------------------------------------------------------------------
// GPU enumeration + selection
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    pub index: u32,
    pub uuid: String,
    pub name: String,
    pub sm: u32,
}

impl GpuInfo {
    pub fn supported(&self) -> bool {
        SUPPORTED_SM.contains(&self.sm)
    }
}

/// All NVIDIA GPUs visible to `nvidia-smi`, in its enumeration order. Under Flatpak
/// this runs on the host (the sandbox has no `nvidia-smi` binary).
pub fn list_gpus() -> Vec<GpuInfo> {
    let out = crate::host_command("nvidia-smi")
        .args([
            "--query-gpu=index,uuid,name,compute_cap",
            "--format=csv,noheader,nounits",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(parse_gpu_row)
            .collect(),
        _ => Vec::new(),
    }
}

/// Parse one `index, uuid, name, compute_cap` CSV row (name may contain commas).
fn parse_gpu_row(line: &str) -> Option<GpuInfo> {
    let parts: Vec<&str> = line.split(',').map(str::trim).collect();
    if parts.len() < 4 {
        return None;
    }
    let index = parts[0].parse::<u32>().ok()?;
    let uuid = parts[1].to_string();
    let name = parts[2..parts.len() - 1].join(", ");
    let (maj, min) = parts[parts.len() - 1].split_once('.')?;
    let sm = maj.trim().parse::<u32>().ok()? * 10 + min.trim().parse::<u32>().ok()?;
    Some(GpuInfo {
        index,
        uuid,
        name,
        sm,
    })
}

use crate::data_home;

fn gpu_pref_path() -> PathBuf {
    data_home().join("hush/gpu")
}

/// UUID of the GPU the user picked (persisted), if any.
pub fn selected_gpu_uuid() -> Option<String> {
    std::fs::read_to_string(gpu_pref_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn persist_gpu_uuid(uuid: &str) {
    let p = gpu_pref_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, uuid);
}

/// The GPU to run on: the persisted pick if it still exists, else GPU 0.
pub fn effective_gpu(gpus: &[GpuInfo]) -> Option<&GpuInfo> {
    if let Some(uuid) = selected_gpu_uuid()
        && let Some(g) = gpus.iter().find(|g| g.uuid == uuid)
    {
        return Some(g);
    }
    gpus.first()
}

// ---------------------------------------------------------------------------
// Resolution (no network — safe for the daemon)
// ---------------------------------------------------------------------------

fn cache_home() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
}

fn cache_dir(sm: u32) -> PathBuf {
    cache_home().join(format!("hush/models/sm_{sm}"))
}

/// Pre-XDG-cache download location; migrated from on resolve.
fn legacy_cache_dir(sm: u32) -> PathBuf {
    data_home().join(format!("hush/models/sm_{sm}"))
}

/// Model as shipped inside a locally-extracted NGC SDK (license-clean, trusted).
/// Model as shipped inside a resolved SDK root (`<sdk>/features/denoiser/models/sm_XX/`).
/// The CDN SDK tarball carries no models, so this only hits for a local NGC SDK.
fn sdk_local(sm: u32, file: &str) -> Option<PathBuf> {
    Some(crate::sdk::sdk_root()?.join(format!("features/denoiser/models/sm_{sm}/{file}")))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

/// Size gate, then sha256 — a cached file is only trusted if both match.
fn verified(path: &Path, entry: &ModelEntry) -> bool {
    match std::fs::metadata(path) {
        Ok(m) if m.len() == entry.size => {}
        _ => return false,
    }
    sha256_file(path)
        .map(|h| h.eq_ignore_ascii_case(entry.sha256))
        .unwrap_or(false)
}

/// Resolve a usable model path WITHOUT downloading. `Ok(None)` = "needs download".
/// Order: `NVAFX_MODEL` > local SDK (trusted) > integrity-verified cache. A cached
/// file that fails verification is evicted and treated as missing.
pub fn resolve_model(version: u32, sm: u32) -> Result<Option<PathBuf>> {
    if let Some(p) = std::env::var_os("NVAFX_MODEL") {
        return Ok(Some(PathBuf::from(p)));
    }
    if !SUPPORTED_SM.contains(&sm) {
        bail!(
            "GPU sm_{sm} is unsupported by Maxine AFX (need one of {SUPPORTED_SM:?}); \
             a Turing (sm_75) or newer NVIDIA GPU is required"
        );
    }
    let file = model_file(version);

    if let Some(sdk) = sdk_local(sm, file)
        && sdk.exists()
    {
        return Ok(Some(sdk));
    }

    let cached = cache_dir(sm).join(file);

    // Best-effort migration from the old data-home cache location.
    if !cached.exists() {
        let legacy = legacy_cache_dir(sm).join(file);
        if legacy.exists() {
            if let Some(d) = cached.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::rename(&legacy, &cached);
        }
    }

    if cached.exists() {
        match manifest_entry(version, sm) {
            Some(e) if verified(&cached, e) => return Ok(Some(cached)),
            _ => {
                let _ = std::fs::remove_file(&cached); // evict corrupt / unverifiable
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Download (GUI-only; run on a dedicated std::thread — reqwest::blocking panics
// inside a tokio context)
// ---------------------------------------------------------------------------

/// Stream the model to `<file>.part`, hashing as it goes, verify against the
/// manifest, then atomically rename into the cache. `progress(done, total)` is
/// called per chunk. Any failure leaves no non-`.part` file behind.
pub fn download_model(
    version: u32,
    sm: u32,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<PathBuf> {
    let entry = manifest_entry(version, sm)
        .with_context(|| format!("no model in the manifest for sm_{sm} v{version}"))?;
    let file = model_file(version);
    let dir = cache_dir(sm);
    std::fs::create_dir_all(&dir).context("create model cache dir")?;
    let dest = dir.join(file);
    let tmp = dir.join(format!("{file}.part"));
    let url = format!("{}/models/sm_{sm}/{file}", cdn_base());

    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .context("build http client")?;
    let mut resp = client
        .get(&url)
        .send()
        .with_context(|| format!("request {url}"))?
        .error_for_status()
        .with_context(|| format!("model URL returned an error: {url}"))?;
    let total = resp.content_length().unwrap_or(entry.size);

    let mut out = std::fs::File::create(&tmp).context("create temp file")?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done: u64 = 0;
    progress(0, total);
    loop {
        let n = resp.read(&mut buf).context("read model stream")?;
        if n == 0 {
            break;
        }
        use std::io::Write;
        out.write_all(&buf[..n]).context("write model")?;
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    drop(out);

    let got = hex(&hasher.finalize());
    let ok = got.eq_ignore_ascii_case(entry.sha256) && done == entry.size;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        bail!(
            "model integrity check failed for sm_{sm} — expected sha256 {} ({} bytes), \
             got {got} ({done} bytes)",
            entry.sha256,
            entry.size
        );
    }
    std::fs::rename(&tmp, &dest).context("finalize downloaded model")?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gpu_rows() {
        let g = parse_gpu_row("0, GPU-abc123, NVIDIA GeForce RTX 4090, 8.9").unwrap();
        assert_eq!(g.index, 0);
        assert_eq!(g.uuid, "GPU-abc123");
        assert_eq!(g.name, "NVIDIA GeForce RTX 4090");
        assert_eq!(g.sm, 89);
        // a comma inside the name must not break parsing
        let g2 = parse_gpu_row("1, GPU-x, NVIDIA RTX A6000, Ada, 8.9").unwrap();
        assert_eq!(g2.name, "NVIDIA RTX A6000, Ada");
        assert_eq!(g2.sm, 89);
        assert!(parse_gpu_row("garbage").is_none());
    }

    #[test]
    fn manifest_is_complete() {
        for &sm in &SUPPORTED_SM {
            assert!(manifest_entry(1, sm).is_some(), "missing v1 sm_{sm}");
            assert!(manifest_entry(2, sm).is_some(), "missing v2 sm_{sm}");
        }
    }

    /// End-to-end download/verify/resolve/evict against a local mirror. Gated on
    /// `HUSH_TEST_DOWNLOAD=1` with `HUSH_MODEL_BASE`, a fake `HOME` and
    /// `XDG_CACHE_HOME` set by the caller (so no SDK path shadows the cache).
    #[test]
    fn download_verify_resolve_evict() {
        if std::env::var("HUSH_TEST_DOWNLOAD").as_deref() != Ok("1") {
            return;
        }
        let (ver, sm) = (2u32, 89u32);
        let _ = std::fs::remove_dir_all(cache_dir(sm));
        assert!(
            matches!(resolve_model(ver, sm), Ok(None)),
            "no model expected pre-download"
        );

        let p = download_model(ver, sm, &|_, _| {}).expect("download should succeed");
        assert!(p.exists());
        let e = manifest_entry(ver, sm).unwrap();
        assert!(
            verified(&p, e),
            "downloaded file must match the manifest hash"
        );
        assert!(
            matches!(resolve_model(ver, sm), Ok(Some(_))),
            "resolve must find verified cache"
        );

        std::fs::write(&p, b"corrupt").unwrap();
        assert!(
            matches!(resolve_model(ver, sm), Ok(None)),
            "corrupt cache must be evicted"
        );
        assert!(!p.exists(), "evicted file must be deleted");
    }
}
