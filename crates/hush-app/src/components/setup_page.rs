//! First-run / recovery page: pick the GPU, download its Maxine model, start up.

use std::sync::atomic::Ordering;
use std::time::Duration;

use dioxus::prelude::*;
use hush_core::model;

use crate::service::restart_engine;
use crate::state::{
    DL_DONE, DL_ERROR, DL_FAILED, DL_GOT, DL_IDLE, DL_PHASE, DL_RUNNING, DL_STAGE, DL_TOTAL,
    ENGINE_ERROR, SETUP_REQUESTED, STAGE_MODEL, STAGE_SDK, STAGE_UNPACK,
};

/// First-run / recovery page: pick the GPU, download its Maxine model, start up.
#[component]
pub(crate) fn SetupPage() -> Element {
    let gpus = use_hook(model::list_gpus);
    // reset a stale DONE/FAILED phase when re-entering setup (keep an in-flight run)
    use_hook(|| {
        if DL_PHASE.load(Ordering::Relaxed) != DL_RUNNING {
            DL_PHASE.store(DL_IDLE, Ordering::Relaxed);
        }
    });
    let mut sel = use_signal(|| {
        model::selected_gpu_uuid()
            .filter(|u| gpus.iter().any(|g| &g.uuid == u))
            .or_else(|| gpus.first().map(|g| g.uuid.clone()))
    });

    let mut phase = use_signal(|| DL_PHASE.load(Ordering::Relaxed));
    let mut stage = use_signal(|| DL_STAGE.load(Ordering::Relaxed));
    let mut got = use_signal(|| 0u64);
    let mut total = use_signal(|| 0u64);
    let mut dl_err = use_signal(|| None::<String>);
    let mut eng_err = use_signal(|| None::<String>);
    use_future(move || async move {
        loop {
            phase.set(DL_PHASE.load(Ordering::Relaxed));
            stage.set(DL_STAGE.load(Ordering::Relaxed));
            got.set(DL_GOT.load(Ordering::Relaxed));
            total.set(DL_TOTAL.load(Ordering::Relaxed));
            dl_err.set(DL_ERROR.lock().ok().and_then(|g| g.clone()));
            eng_err.set(ENGINE_ERROR.lock().ok().and_then(|g| g.clone()));
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    });

    let ph = phase();
    let cur = gpus.iter().find(|g| Some(&g.uuid) == sel().as_ref());
    let sm_val = cur.map(|g| g.sm).unwrap_or(0);
    let supported = cur.map(|g| g.supported()).unwrap_or(false);
    let sdk_ready = hush_core::sdk::sdk_ready();
    let model_mb = model::manifest_entry(2, sm_val)
        .map(|e| e.size / (1024 * 1024))
        .unwrap_or(0);
    let sdk_mb = hush_core::sdk::RUNTIME.size / (1024 * 1024);
    let total_dl_mb = model_mb + if sdk_ready { 0 } else { sdk_mb };
    let stage_label = match stage() {
        STAGE_SDK => "NVIDIA runtime",
        STAGE_UNPACK => "Unpacking runtime",
        _ => "Denoiser model",
    };
    let t = total();
    let pct = if t > 0 {
        (got() as f64 / t as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    let got_mb = got() / (1024 * 1024);
    let total_mb = t / (1024 * 1024);
    let rows: Vec<(String, String, u32, bool, bool)> = gpus
        .iter()
        .map(|g| {
            let on = sel().as_deref() == Some(g.uuid.as_str());
            (g.uuid.clone(), g.name.clone(), g.sm, g.supported(), on)
        })
        .collect();

    rsx! {
        div { class: "screen",
            div { class: "pageview",
                div { class: "page",
                    div { class: "phead", "SETUP" }
                    div { class: "psub", "HUSH needs NVIDIA's Maxine denoiser model built for your GPU. Pick your card and download it — this happens once." }

                    div { class: "card",
                        div { class: "sett", "Graphics card" }
                        if rows.is_empty() {
                            div { class: "setd", "No NVIDIA GPU detected. HUSH needs an NVIDIA GPU (Turing / RTX 20-series or newer)." }
                        }
                        for (uuid , name , gsm , ok , on) in rows.iter().cloned() {
                            button {
                                key: "{uuid}",
                                class: if on { "gpurow on" } else { "gpurow" },
                                disabled: ph == DL_RUNNING,
                                onclick: move |_| {
                                    model::persist_gpu_uuid(&uuid);
                                    sel.set(Some(uuid.clone()));
                                },
                                div { class: "gpumeta",
                                    div { class: "gpuname", "{name}" }
                                    div { class: "gpuarch",
                                        if ok { "sm_{gsm}" } else { "sm_{gsm} · not supported by Maxine" }
                                    }
                                }
                                div { class: if on { "gpudot on" } else { "gpudot" } }
                            }
                        }
                    }

                    if supported {
                        div { class: "card",
                            if !sdk_ready {
                                div { class: "setrow",
                                    div { class: "setmeta",
                                        div { class: "sett", "NVIDIA runtime" }
                                        div { class: "setd", "Maxine denoiser · CUDA/TensorRT · ~{sdk_mb} MB download, ~3.6 GB installed" }
                                    }
                                }
                            }
                            div { class: "setrow",
                                div { class: "setmeta",
                                    div { class: "sett", "Denoiser model" }
                                    div { class: "setd", "Maxine v2  ·  sm_{sm_val}  ·  {model_mb} MB" }
                                }
                                if ph == DL_DONE {
                                    div { class: "stat on", span { class: "wdot" } "STARTING" }
                                }
                            }
                            if ph == DL_RUNNING || ph == DL_DONE {
                                div { class: "dlbar", div { class: "dlfill", style: "width:{pct:.1}%" } }
                                div { class: "dlnote",
                                    if ph == DL_DONE {
                                        "Verified. Starting the engine…"
                                    } else if stage() == STAGE_UNPACK {
                                        "Unpacking NVIDIA runtime… (this takes a moment)"
                                    } else {
                                        "{stage_label}  ·  {pct:.0}%  ·  {got_mb} / {total_mb} MB"
                                    }
                                }
                            } else {
                                button {
                                    class: "dlbtn",
                                    onclick: move |_| {
                                        DL_GOT.store(0, Ordering::Relaxed);
                                        DL_TOTAL.store(0, Ordering::Relaxed);
                                        if let Ok(mut g) = DL_ERROR.lock() { *g = None; }
                                        DL_PHASE.store(DL_RUNNING, Ordering::Relaxed);
                                        std::thread::spawn(move || {
                                            let progress = |d: u64, t: u64| {
                                                DL_GOT.store(d, Ordering::Relaxed);
                                                DL_TOTAL.store(t, Ordering::Relaxed);
                                            };
                                            // SDK first (if missing), then the per-GPU model.
                                            let r = (|| -> anyhow::Result<()> {
                                                if !hush_core::sdk::sdk_ready() {
                                                    DL_STAGE.store(STAGE_SDK, Ordering::Relaxed);
                                                    hush_core::sdk::download_sdk(&|d, t| {
                                                        // last chunk → flip to "unpacking" while extraction runs
                                                        if t > 0 && d >= t { DL_STAGE.store(STAGE_UNPACK, Ordering::Relaxed); }
                                                        progress(d, t);
                                                    })?;
                                                }
                                                DL_STAGE.store(STAGE_MODEL, Ordering::Relaxed);
                                                progress(0, 0);
                                                model::download_model(2, sm_val, &progress)?;
                                                Ok(())
                                            })();
                                            match r {
                                                Ok(_) => {
                                                    SETUP_REQUESTED.store(false, Ordering::Relaxed);
                                                    DL_PHASE.store(DL_DONE, Ordering::Relaxed);
                                                    restart_engine();
                                                }
                                                Err(e) => {
                                                    if let Ok(mut g) = DL_ERROR.lock() { *g = Some(e.to_string()); }
                                                    DL_PHASE.store(DL_FAILED, Ordering::Relaxed);
                                                }
                                            }
                                        });
                                    },
                                    if ph == DL_FAILED { "Retry" } else { "Download & set up  ·  {total_dl_mb} MB" }
                                }
                            }
                            if ph == DL_FAILED {
                                if let Some(e) = dl_err() {
                                    div { class: "dlerr", "{e}" }
                                }
                            }
                        }
                    }

                    if let Some(e) = eng_err() {
                        div { class: "card",
                            div { class: "sett", "Engine problem" }
                            div { class: "dlerr", "{e}" }
                        }
                    }
                }
            }
        }
    }
}
