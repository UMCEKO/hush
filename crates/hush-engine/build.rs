use std::path::Path;

// Links the daemon against NVIDIA's AFX runtime (libnv_audiofx + libcudart). We
// never vendor/redistribute the SDK — the linker is pointed at libs provided one
// of two ways:
//
//   NVAFX_LINK_DIR : a flat dir (from the CDN afx-link tarball) with
//                    nvafx/lib + external/cuda/lib. Used by packaged builds
//                    (CI / AUR / Nix). NO rpath is baked — the packaged daemon
//                    resolves libs at runtime via LD_LIBRARY_PATH (set by the GUI
//                    from the provisioned SDK).
//   NVAFX_SDK      : a full extracted SDK root (dev convenience). Bakes DT_RPATH
//                    into the SDK's three lib dirs so `cargo run` just works.
//   (default)      : ~/maxine-dl/sdk/Audio_Effects_SDK, treated like NVAFX_SDK.
fn main() {
    if let Ok(link_dir) = std::env::var("NVAFX_LINK_DIR") {
        let nvafx_lib = format!("{link_dir}/nvafx/lib");
        let cuda_lib = format!("{link_dir}/external/cuda/lib");
        ensure_symlinks(&nvafx_lib);
        println!("cargo:rustc-link-search=native={nvafx_lib}");
        println!("cargo:rustc-link-search=native={cuda_lib}");
        println!("cargo:rustc-link-lib=dylib=nv_audiofx");
        println!("cargo:rustc-link-lib=dylib=cudart");
        // No rpath: packaged runtime uses LD_LIBRARY_PATH.
        return;
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let sdk = std::env::var("NVAFX_SDK")
        .unwrap_or_else(|_| format!("{home}/maxine-dl/sdk/Audio_Effects_SDK"));
    let nvafx_lib = format!("{sdk}/nvafx/lib");
    let cuda_lib = format!("{sdk}/external/cuda/lib");
    let feat_lib = format!("{sdk}/features/denoiser/lib");

    ensure_symlinks(&nvafx_lib);
    println!("cargo:rustc-link-search=native={nvafx_lib}");
    println!("cargo:rustc-link-search=native={cuda_lib}");
    println!("cargo:rustc-link-lib=dylib=nv_audiofx");
    println!("cargo:rustc-link-lib=dylib=cudart");
    // DT_RPATH (not RUNPATH) so the dlopen'd feature lib + TRT deps resolve for dev runs.
    println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
    for d in [&nvafx_lib, &cuda_lib, &feat_lib] {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{d}");
    }
}

// The SDK ships only libnv_audiofx.so.2.1.0; create the unversioned (-l) and
// SONAME (.so.2) symlinks the linker expects, next to it (if writable).
fn ensure_symlinks(nvafx_lib: &str) {
    let real = format!("{nvafx_lib}/libnv_audiofx.so.2.1.0");
    for name in ["libnv_audiofx.so", "libnv_audiofx.so.2"] {
        let link = format!("{nvafx_lib}/{name}");
        if !Path::new(&link).exists() {
            let _ = std::os::unix::fs::symlink(&real, &link);
        }
    }
}
