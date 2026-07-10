//! Headless CLI: run the HUSH virtual mic engine directly (dev tool).
//!   mic [intensity 0-100] [version 1|2]

use std::sync::{Arc, Mutex};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let pct: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let version: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);

    let sm = hush_core::model::list_gpus()
        .first()
        .map(|g| g.sm)
        .ok_or_else(|| anyhow::anyhow!("no NVIDIA GPU detected"))?;
    let model = hush_core::model::resolve_model(version, sm)?
        .ok_or_else(|| anyhow::anyhow!("no model for sm_{sm}; run the GUI setup first"))?;

    let controls = hush_core::Controls::new();
    controls.set_intensity(pct.min(100) as f32 / 100.0);
    println!("HUSH engine: intensity {pct}%, model v{version}");
    hush_engine::engine::run(version, model, controls, Arc::new(Mutex::new(None)))
}
