//! hushd — the HUSH denoiser daemon.
//!
//! Owns the Maxine engine + the `HUSH` virtual mic and serves live control +
//! spectrum over a Unix socket. The GUI is just a client: it can attach and detach
//! (or crash) without ever interrupting the audio. Meant to run as a systemd *user*
//! service (PipeWire is session-scoped, so this can't be a system/root service).
//!
//! The daemon never downloads models — it only *resolves* one (env > SDK > verified
//! cache). If none is available it stays up and reports `model_missing` so the GUI
//! can prompt the user to download it, instead of crash-looping under systemd.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use hush_core::ipc::{ClientMsg, StateFrame, socket_path};
use hush_core::{Controls, SPECTRUM_BINS, model};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

const MODEL_VERSION: u32 = 2;

/// Engine/model status shared with every connected GUI via `StateFrame`.
struct DaemonState {
    model_missing: AtomicBool,
    engine_error: Arc<Mutex<Option<String>>>,
    gpu_name: Option<String>,
}

fn main() -> Result<()> {
    // Pick the GPU and pin CUDA to it BEFORE anything touches CUDA. This is the
    // first statement in a still-single-threaded `main`, so `set_var` is sound and
    // the engine's later CUDA/TensorRT init lands on the user's selected device.
    let gpus = model::list_gpus();
    let gpu = model::effective_gpu(&gpus).cloned();
    if let Some(g) = &gpu {
        unsafe { std::env::set_var("CUDA_VISIBLE_DEVICES", &g.uuid) };
    }

    let state = Arc::new(DaemonState {
        model_missing: AtomicBool::new(false),
        engine_error: Arc::new(Mutex::new(None)),
        gpu_name: gpu.as_ref().map(|g| g.name.clone()),
    });
    let controls = Controls::new();

    // Resolve the model for the selected GPU's arch — no network here.
    match gpu.as_ref().map(|g| model::resolve_model(MODEL_VERSION, g.sm)) {
        Some(Ok(Some(path))) => {
            let controls = controls.clone();
            let err_sink = state.engine_error.clone();
            std::thread::spawn(move || {
                if let Err(e) = hush_engine::engine::run(MODEL_VERSION, path, controls, err_sink) {
                    // PipeWire/audio setup failure — exit so systemd retries it.
                    eprintln!("hushd: engine error: {e}");
                    std::process::exit(1);
                }
            });
        }
        Some(Ok(None)) => {
            eprintln!("hushd: no model for this GPU yet — waiting for the GUI to download one");
            state.model_missing.store(true, Ordering::Relaxed);
        }
        Some(Err(e)) => {
            eprintln!("hushd: {e}");
            *state.engine_error.lock().unwrap() = Some(e.to_string());
        }
        None => {
            let msg = "no NVIDIA GPU detected (is nvidia-smi available?)";
            eprintln!("hushd: {msg}");
            *state.engine_error.lock().unwrap() = Some(msg.to_string());
        }
    }

    // Serve the control socket regardless of engine state.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(serve(controls, state))
}

async fn serve(controls: Arc<Controls>, state: Arc<DaemonState>) -> Result<()> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener = UnixListener::bind(&path)?;
    eprintln!("hushd: listening on {}", path.display());

    loop {
        let (stream, _) = listener.accept().await?;
        let controls = controls.clone();
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_client(stream, controls, state).await {
                eprintln!("hushd: client disconnected: {e}");
            }
        });
    }
}

/// One GUI connection: apply its control commands, stream state back at ~30 Hz.
async fn serve_client(
    stream: UnixStream,
    controls: Arc<Controls>,
    state: Arc<DaemonState>,
) -> Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut lines = BufReader::new(rd).lines();
    let mut tick = tokio::time::interval(Duration::from_millis(33));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let spectrum = controls
                    .spectrum
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_else(|_| vec![0.0; SPECTRUM_BINS]);
                let spectrum_in = controls
                    .spectrum_in
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_else(|_| vec![0.0; SPECTRUM_BINS]);
                let frame = StateFrame {
                    intensity: controls.intensity(),
                    notches: controls.notches_snapshot(),
                    model_version: MODEL_VERSION,
                    spectrum,
                    spectrum_in,
                    model_missing: state.model_missing.load(Ordering::Relaxed),
                    engine_error: state.engine_error.lock().ok().and_then(|g| g.clone()),
                    gpu_name: state.gpu_name.clone(),
                };
                let mut buf = serde_json::to_vec(&frame)?;
                buf.push(b'\n');
                if wr.write_all(&buf).await.is_err() {
                    break;
                }
            }
            line = lines.next_line() => {
                match line? {
                    Some(line) => apply(&line, &controls),
                    None => break,
                }
            }
        }
    }
    Ok(())
}

fn apply(line: &str, controls: &Controls) {
    match serde_json::from_str::<ClientMsg>(line) {
        Ok(ClientMsg::Intensity { value }) => controls.set_intensity(value),
        Ok(ClientMsg::SetNotches { notches }) => controls.set_notches(notches),
        Ok(ClientMsg::Shutdown) => {
            hush_engine::engine::unload_virtual_mic();
            std::process::exit(0);
        }
        Err(_) => {}
    }
}
