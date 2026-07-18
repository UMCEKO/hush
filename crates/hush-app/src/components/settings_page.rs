//! Settings + QoL: mic pick, GPU, launch-at-login, restart, about.

use std::sync::atomic::Ordering;

use dioxus::prelude::*;
use hush_core::model::{self, GpuInfo};

use super::main_page::MicRow;
use crate::service::{restart_engine, service_enabled, set_autostart};
use crate::state::{CLOSE_TO_TRAY, GPU_NAME, SETUP_REQUESTED, persist_close_to_tray};

/// Label for the currently-selected GPU (settings row).
fn current_gpu_label(gpus: &[GpuInfo]) -> String {
    if let Some(g) = model::effective_gpu(gpus) {
        format!("{} · sm_{}", g.name, g.sm)
    } else if let Some(n) = GPU_NAME.lock().ok().and_then(|g| g.clone()) {
        n
    } else {
        "No NVIDIA GPU".into()
    }
}

/// Switch to the next GPU, persist it, and either restart (model present) or route
/// to the setup page (the new arch needs a model download).
fn cycle_gpu(gpus: &[GpuInfo]) {
    if gpus.len() < 2 {
        return;
    }
    let idx = model::selected_gpu_uuid()
        .and_then(|u| gpus.iter().position(|g| g.uuid == u))
        .unwrap_or(0);
    let next = &gpus[(idx + 1) % gpus.len()];
    model::persist_gpu_uuid(&next.uuid);
    match model::resolve_model(2, next.sm) {
        Ok(Some(_)) => restart_engine(),
        _ => SETUP_REQUESTED.store(true, Ordering::Relaxed),
    }
}

/// Settings + QoL: mic pick, GPU, launch-at-login, restart, about.
#[component]
pub(crate) fn SettingsPage() -> Element {
    let mut autostart = use_signal(service_enabled);
    let mut to_tray = use_signal(|| CLOSE_TO_TRAY.load(Ordering::Relaxed));
    let gpus = use_hook(model::list_gpus);
    let mut gpu_label = use_signal({
        let gpus = gpus.clone();
        move || current_gpu_label(&gpus)
    });
    let multi_gpu = gpus.len() > 1;
    rsx! {
        div { class: "page",
            div { class: "phead", "SETTINGS" }
            div { class: "card",
                MicRow {}
                div { class: "setrow",
                    div { class: "setmeta",
                        div { class: "sett", "GPU" }
                        div { class: "setd", "{gpu_label}" }
                    }
                    if multi_gpu {
                        button {
                            class: "sbtn",
                            onclick: move |_| {
                                cycle_gpu(&gpus);
                                gpu_label.set(current_gpu_label(&gpus));
                            },
                            "SWITCH"
                        }
                    }
                }
                // systemd --user isn't reachable inside the Flatpak sandbox; the
                // daemon is sibling-spawned there instead, so hide this toggle.
                if !hush_core::in_flatpak() {
                    div { class: "setrow",
                        div { class: "setmeta",
                            div { class: "sett", "Launch at login" }
                            div { class: "setd", "Start the denoiser with your session" }
                        }
                        button {
                            class: if autostart() { "switch on" } else { "switch" },
                            onclick: move |_| { let n = !autostart(); set_autostart(n); autostart.set(n); },
                            div { class: "sknob" }
                        }
                    }
                }
                div { class: "setrow",
                    div { class: "setmeta",
                        div { class: "sett", "Close to tray" }
                        div { class: "setd", "✕ hides to the tray — the denoiser keeps running" }
                    }
                    button {
                        class: if to_tray() { "switch on" } else { "switch" },
                        onclick: move |_| {
                            let n = !to_tray();
                            CLOSE_TO_TRAY.store(n, Ordering::Relaxed);
                            persist_close_to_tray(n);
                            to_tray.set(n);
                        },
                        div { class: "sknob" }
                    }
                }
                div { class: "setrow",
                    div { class: "setmeta",
                        div { class: "sett", "Restart engine" }
                        div { class: "setd", "Reload the model + virtual mic" }
                    }
                    button { class: "sbtn", onclick: move |_| restart_engine(), "RESTART" }
                }
            }
            div { class: "card",
                div { class: "sett", "About" }
                p { class: "setabout",
                    "HUSH runs NVIDIA's Maxine Audio Effects denoiser on your GPU, used under the NVIDIA Software License Agreement and Open Model License. Not affiliated with, sponsored by, or endorsed by NVIDIA."
                }
                div { class: "setd", "Model  Maxine v2    ·    Engine  hushd    ·    Build  HUSH {env!(\"CARGO_PKG_VERSION\")}" }
            }
        }
    }
}
