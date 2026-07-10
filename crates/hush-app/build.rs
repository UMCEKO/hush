//! Bakes `assets/fonts.css` into `$OUT_DIR/fonts.css` with each `url(fonts/NAME)`
//! reference replaced by a base64 `data:` URI of the font bytes. Adapted from the
//! Kopuz project's build.rs. Lets a plain `cargo build` (no dx) produce a fully
//! styled, network-free GUI binary.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let assets = crate_dir.join("assets");
    let fonts = assets.join("fonts");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));

    let src = assets.join("fonts.css");
    println!("cargo:rerun-if-changed={}", src.display());
    let css = fs::read_to_string(&src).unwrap_or_else(|e| panic!("read {}: {e}", src.display()));
    fs::write(out_dir.join("fonts.css"), bake_font_css(&css, &fonts))
        .expect("write baked fonts.css");
}

fn bake_font_css(css: &str, fonts: &Path) -> String {
    use base64::Engine;
    const PREFIX: &str = "url(fonts/";

    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(idx) = rest.find(PREFIX) {
        out.push_str(&rest[..idx]);
        let after = &rest[idx + PREFIX.len()..];
        let end = after
            .find(')')
            .unwrap_or_else(|| panic!("unterminated `{PREFIX}` in fonts.css"));
        let file = &after[..end];
        let path = fonts.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read font {}: {e}", path.display()));
        let mime = match Path::new(file).extension().and_then(|e| e.to_str()) {
            Some("woff2") => "font/woff2",
            Some("woff") => "font/woff",
            Some("otf") => "font/otf",
            Some("ttf") => "font/ttf",
            _ => "application/octet-stream",
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        out.push_str("url(data:");
        out.push_str(mime);
        out.push_str(";base64,");
        out.push_str(&b64);
        out.push(')');
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}
