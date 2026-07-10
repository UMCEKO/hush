//! Headless CLI: run the NV-Maxine virtual mic engine.
//!   mic [intensity 0-100] [version 1|2]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let pct: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let version: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let controls = nv_maxine::Controls::new();
    controls.set_intensity(pct.min(100) as f32 / 100.0);
    println!("NV-Maxine engine: intensity {pct}%, model v{version}");
    nv_maxine::engine::run(version, controls)
}
