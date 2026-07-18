//! GUI↔daemon sync over the control socket: reconnecting mirror thread, daemon
//! autostart, and the delta-send/frame-receive loop.

use std::sync::atomic::Ordering;
use std::time::Duration;

use std::os::unix::process::CommandExt;

use hush_core::Controls;
use hush_core::ipc::{ClientMsg, StateFrame, socket_path};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::state::{
    ACTIVE_MIC, CONNECTED, ENGINE_ERROR, GPU_NAME, MIC_TOUCHED, MICS, MODEL_MISSING,
};

/// Background thread that keeps the GUI's `CONTROLS` mirror in sync with `hushd`:
/// pushes local intensity/notch changes to the daemon and pulls its spectrum back.
/// Reconnects on its own, so the daemon and GUI lifecycles are fully independent.
pub(crate) fn spawn_ipc_sync(mirror: std::sync::Arc<Controls>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            loop {
                if let Ok(stream) = connect_daemon().await {
                    CONNECTED.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = run_sync(stream, &mirror).await;
                    CONNECTED.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(600)).await;
            }
        });
    });
}

/// Connect to `hushd`, starting it if it isn't up yet.
async fn connect_daemon() -> std::io::Result<UnixStream> {
    let path = socket_path();
    if let Ok(stream) = UnixStream::connect(&path).await {
        return Ok(stream);
    }
    start_daemon();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if let Ok(stream) = UnixStream::connect(&path).await {
            return Ok(stream);
        }
    }
    UnixStream::connect(&path).await
}

/// Bring the daemon up: prefer the systemd user service, else spawn a sibling
/// `hushd` binary detached into its own process group so it outlives the GUI.
fn start_daemon() {
    // The daemon links the SDK at exec — don't try to start it before the runtime
    // is provisioned, or it just fails to load and (under systemd) crash-loops.
    let ld = match hush_core::sdk::ld_library_path() {
        Some(p) => p,
        None => return,
    };
    let via_systemd = std::process::Command::new("systemctl")
        .args(["--user", "start", "hush.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if via_systemd {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let hushd = exe.with_file_name("hushd");
        let _ = std::process::Command::new(hushd)
            .env("LD_LIBRARY_PATH", ld)
            .process_group(0)
            .spawn();
    }
}

/// Ask the daemon to exit cleanly (its supervisor / the GUI reconnect loop brings
/// it back). Used when there's no systemd unit to `restart`.
pub(crate) fn send_shutdown() {
    use std::io::Write;
    if let Ok(mut s) = std::os::unix::net::UnixStream::connect(socket_path())
        && let Ok(mut buf) = serde_json::to_vec(&ClientMsg::Shutdown)
    {
        buf.push(b'\n');
        let _ = s.write_all(&buf);
    }
}

/// Drive one daemon connection until it drops: send control deltas, receive frames.
async fn run_sync(stream: UnixStream, mirror: &Controls) -> std::io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut lines = BufReader::new(rd).lines();
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    // Force an initial push so the daemon adopts this GUI's current settings.
    let mut last_intensity = f32::NAN;
    let mut last_gen = u64::MAX;
    // Mic is the other way round: the daemon owns the persisted choice, so only
    // push it when the user actually touched the picker.
    let mut last_mic_gen = if MIC_TOUCHED.load(Ordering::Relaxed) {
        u64::MAX
    } else {
        mirror.mic_gen()
    };

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let intensity = mirror.intensity();
                if intensity != last_intensity {
                    last_intensity = intensity;
                    if !send(&mut wr, ClientMsg::Intensity { value: intensity }).await { break; }
                }
                let generation = mirror.notch_gen();
                if generation != last_gen {
                    last_gen = generation;
                    let notches = mirror.notches_snapshot();
                    if !send(&mut wr, ClientMsg::SetNotches { notches }).await { break; }
                }
                let mic_gen = mirror.mic_gen();
                if mic_gen != last_mic_gen {
                    last_mic_gen = mic_gen;
                    if !send(&mut wr, ClientMsg::SetMic { name: mirror.mic() }).await { break; }
                }
            }
            line = lines.next_line() => {
                match line? {
                    Some(line) => {
                        if let Ok(frame) = serde_json::from_str::<StateFrame>(&line) {
                            if let Ok(mut g) = mirror.spectrum.lock() {
                                *g = frame.spectrum;
                            }
                            if let Ok(mut g) = mirror.spectrum_in.lock() {
                                *g = frame.spectrum_in;
                            }
                            MODEL_MISSING.store(frame.model_missing, Ordering::Relaxed);
                            if let Ok(mut g) = ENGINE_ERROR.lock() {
                                *g = frame.engine_error.clone();
                            }
                            if let Ok(mut g) = GPU_NAME.lock() {
                                *g = frame.gpu_name.clone();
                            }
                            if let Ok(mut g) = MICS.lock() {
                                *g = frame.mics.clone();
                            }
                            if let Ok(mut g) = ACTIVE_MIC.lock() {
                                *g = frame.mic.clone();
                            }
                        }
                    }
                    None => break,
                }
            }
        }
    }
    Ok(())
}

async fn send(wr: &mut (impl AsyncWriteExt + Unpin), msg: ClientMsg) -> bool {
    match serde_json::to_vec(&msg) {
        Ok(mut buf) => {
            buf.push(b'\n');
            wr.write_all(&buf).await.is_ok()
        }
        Err(_) => true,
    }
}
