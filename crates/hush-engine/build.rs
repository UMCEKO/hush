use std::path::Path;

// Links against NVIDIA's AFX runtime (libnv_audiofx) from the SDK the user
// downloaded. We do NOT vendor/redistribute it — just point the linker at it.
fn main() {
    let home = std::env::var("HOME").unwrap();
    let sdk = std::env::var("NVAFX_SDK")
        .unwrap_or_else(|_| format!("{home}/maxine-dl/sdk/Audio_Effects_SDK"));
    let nvafx_lib = format!("{sdk}/nvafx/lib");
    let cuda_lib = format!("{sdk}/external/cuda/lib");
    let feat_lib = format!("{sdk}/features/denoiser/lib");

    // The SDK ships only libnv_audiofx.so.2.1.0; create the unversioned (-l) and
    // SONAME (.so.2) symlinks the linker/loader expect, next to it.
    let real = format!("{nvafx_lib}/libnv_audiofx.so.2.1.0");
    for name in ["libnv_audiofx.so", "libnv_audiofx.so.2"] {
        let link = format!("{nvafx_lib}/{name}");
        if !Path::new(&link).exists() {
            let _ = std::os::unix::fs::symlink(&real, &link);
        }
    }

    println!("cargo:rustc-link-search=native={nvafx_lib}");
    println!("cargo:rustc-link-search=native={cuda_lib}");
    println!("cargo:rustc-link-lib=dylib=nv_audiofx");
    println!("cargo:rustc-link-lib=dylib=cudart");
    // DT_RPATH (not RUNPATH) so the dlopen'd feature lib + TRT deps resolve too.
    println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
    for d in [&nvafx_lib, &cuda_lib, &feat_lib] {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{d}");
    }
}
