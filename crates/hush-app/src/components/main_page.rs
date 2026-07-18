//! Main "POWER" page: the big toggle, suppression strength, engine status, and
//! the capture-device picker.

use std::sync::atomic::Ordering;
use std::time::Duration;

use dioxus::prelude::*;
use hush_core::ipc::MicInfo;

use crate::state::{ACTIVE_MIC, CONNECTED, CONTROLS, Ctl, MIC_TOUCHED, MICS};

/// WARP-style main page: one huge toggle + the suppression strength.
#[component]
pub(crate) fn MainPage() -> Element {
    let ctl = use_context::<Ctl>();
    let mut power = ctl.power;
    let mut level = ctl.level;
    let on = power();

    // Track the daemon connection so the toggle can warn when it's switched on
    // with no engine behind it.
    let mut connected = use_signal(|| CONNECTED.load(Ordering::Relaxed));
    use_future(move || async move {
        loop {
            connected.set(CONNECTED.load(Ordering::Relaxed));
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    });
    // On but the engine isn't attached → the switch means nothing yet: warn (red).
    let stalled = on && !connected();

    rsx! {
        div { class: "page",
            div { class: "warpwrap",
                div {
                    class: if stalled { "wbadge warn" } else if on { "wbadge on" } else { "wbadge" },
                    span { class: "wdot" }
                    if stalled { "NO ENGINE" } else if on { "ACTIVE" } else { "BYPASS" }
                }
                button {
                    class: if stalled { "warp warn" } else if on { "warp on" } else { "warp" },
                    onclick: move |_| power.toggle(),
                    // Power glyph drawn purely in CSS (.warp-knob::after mask) — a ⏻ text
                    // node renders as an uncenterable color-emoji keycap, and an inline
                    // <svg> child gets mis-styled by this dioxus alpha's attr handling.
                    div { class: "warp-knob" }
                }
                div { class: "wtitle",
                    if stalled { "Engine offline" } else if on { "Noise suppression on" } else { "Passing through raw" }
                }
                div { class: "wsub",
                    if stalled { "hushd isn't running — start it from Settings → Restart engine, or finish setup." }
                    else if on { "Your microphone is cleaned on the GPU in real time." }
                    else { "HUSH is idle — your mic is unprocessed." }
                }
            }
            div { class: "card",
                div { class: "mrow",
                    span { class: "label", "SUPPRESSION" }
                    span { class: "mval", "{level}%" }
                }
                input {
                    class: "fader", style: "--pct: {level}%",
                    r#type: "range", min: "0", max: "100", value: "{level}",
                    disabled: !on,
                    oninput: move |e| { if let Ok(v) = e.value().parse::<u32>() { level.set(v); } }
                }
                div { class: "scale", span { "RAW" } span { "MAX" } }
            }
            div { class: "card",
                div { class: "setrow",
                    div { class: "setmeta",
                        div { class: "sett", "Engine" }
                        div { class: "setd", "hushd denoiser service" }
                    }
                    div { class: if connected() { "stat on" } else { "stat" },
                        span { class: "wdot" }
                        if connected() { "CONNECTED" } else { "OFFLINE" }
                    }
                }
                MicRow {}
            }
            div { class: "foot",
                span { class: "fdot" }
                "Select  HUSH  as your microphone in any app."
            }
        }
    }
}

/// Capture-device picker (shared by Power + Settings): lists the daemon's
/// enumerated sources and applies a pick live — the daemon persists it and
/// reroutes the capture stream without an engine restart.
#[component]
pub(crate) fn MicRow() -> Element {
    let mut mics = use_signal(Vec::<MicInfo>::new);
    let mut active = use_signal(|| ACTIVE_MIC.lock().ok().and_then(|g| g.clone()));
    use_future(move || async move {
        loop {
            mics.set(MICS.lock().map(|g| g.clone()).unwrap_or_default());
            active.set(ACTIVE_MIC.lock().ok().and_then(|g| g.clone()));
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
    });
    let cur = active();
    let missing = cur
        .as_ref()
        .filter(|c| !mics().iter().any(|m| &&m.name == c))
        .cloned();
    rsx! {
        div { class: "setrow",
            div { class: "setmeta",
                div { class: "sett", "Microphone" }
                div { class: "setd", "The input HUSH cleans" }
            }
            select {
                class: "mselect",
                onchange: move |e| {
                    let v = e.value();
                    let name = (v != "__default").then_some(v);
                    MIC_TOUCHED.store(true, Ordering::Relaxed);
                    active.set(name.clone());
                    if let Ok(mut g) = ACTIVE_MIC.lock() {
                        *g = name.clone();
                    }
                    if let Some(c) = CONTROLS.get() {
                        c.set_mic(name);
                    }
                },
                option { value: "__default", selected: cur.is_none(), "System default" }
                if let Some(m) = missing {
                    option { value: "{m}", selected: true, "{m} (unplugged)" }
                }
                for m in mics() {
                    option {
                        value: "{m.name}",
                        selected: cur.as_deref() == Some(m.name.as_str()),
                        "{m.desc}"
                    }
                }
            }
        }
    }
}
