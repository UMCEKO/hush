//! HUSH — noise-suppression virtual mic. Control-deck UI over the live Maxine
//! engine, with a first-run license screen. Webview default; `NV_MAXINE_BLITZ=1`
//! uses the native Blitz/wgpu renderer (build with `--features blitz`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use std::os::unix::process::CommandExt;

use dioxus::prelude::*;
use hush_core::ipc::{ClientMsg, StateFrame, socket_path};
use hush_core::model::{self, GpuInfo};
use hush_core::{Controls, NotchParam, SPECTRUM_BIN_HZ, SPECTRUM_BINS};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

static CONTROLS: OnceLock<Arc<Controls>> = OnceLock::new();
/// True while the GUI is attached to a live `hushd`.
static CONNECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// When set, the ✕ button hides the window to the tray instead of quitting.
static CLOSE_TO_TRAY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Pinged when another launch — or the tray "Show" item — asks us to surface.
/// Event-driven (no polling); `notify_one` stores a permit if the UI isn't yet
/// awaiting, so a wake-up is never lost or raced.
static SHOW_NOTIFY: tokio::sync::Notify = tokio::sync::Notify::const_new();

// ---- setup / model-provisioning state, mirrored from the daemon's StateFrame ----
/// The daemon has no usable model — route the GUI to the setup/download page.
static MODEL_MISSING: AtomicBool = AtomicBool::new(false);
/// Engine init failed (bad model, no GPU, …); shown on the setup page.
static ENGINE_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// GPU the daemon reports running on (for the settings row).
static GPU_NAME: Mutex<Option<String>> = Mutex::new(None);
/// Set by the settings GPU picker to force the setup page (new arch needs a model).
static SETUP_REQUESTED: AtomicBool = AtomicBool::new(false);

// ---- live model-download progress (written by the download thread) ----
const DL_IDLE: u8 = 0;
const DL_RUNNING: u8 = 1;
const DL_DONE: u8 = 2;
const DL_FAILED: u8 = 3;
static DL_PHASE: AtomicU8 = AtomicU8::new(DL_IDLE);
static DL_GOT: AtomicU64 = AtomicU64::new(0);
static DL_TOTAL: AtomicU64 = AtomicU64::new(0);
static DL_ERROR: Mutex<Option<String>> = Mutex::new(None);
// Which artifact is in flight, for the progress label.
const STAGE_SDK: u8 = 0;
const STAGE_MODEL: u8 = 1;
const STAGE_UNPACK: u8 = 2;
static DL_STAGE: AtomicU8 = AtomicU8::new(STAGE_SDK);

fn ctray_path() -> PathBuf {
    hush_core::data_home().join("hush/close_to_tray")
}
fn load_close_to_tray() -> bool {
    ctray_path().exists()
}
fn persist_close_to_tray(on: bool) {
    let p = ctray_path();
    if on {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, "1");
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Are we actually running the Blitz renderer (vs the default webview)?
fn running_blitz() -> bool {
    cfg!(feature = "blitz") && blitz_on()
}

fn flag_path() -> PathBuf {
    hush_core::data_home().join("hush/eula_accepted")
}
fn eula_accepted() -> bool {
    flag_path().exists()
}
fn persist_accept() {
    let p = flag_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, "1");
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Main,
    Freq,
    Settings,
}

/// Shared live control state, provided via context to every page.
#[derive(Clone, Copy)]
struct Ctl {
    power: Signal<bool>,
    level: Signal<u32>,
    notches: Signal<Vec<NotchParam>>,
    spectrum: Signal<Vec<f32>>,    // adjusted output
    spectrum_in: Signal<Vec<f32>>, // original mic
}

/// Whether to route to the setup page: the SDK runtime isn't provisioned yet (the
/// daemon can't even exec without it), the daemon reports no model, an engine
/// error, or the settings GPU picker asked for it.
fn setup_needed() -> bool {
    !hush_core::sdk::sdk_ready()
        || MODEL_MISSING.load(Ordering::Relaxed)
        || SETUP_REQUESTED.load(Ordering::Relaxed)
        || ENGINE_ERROR
            .lock()
            .ok()
            .map(|g| g.is_some())
            .unwrap_or(false)
}

#[component]
fn App() -> Element {
    let accepted = use_signal(eula_accepted);
    let mut needs_setup = use_signal(|| false);
    use_future(move || async move {
        loop {
            let n = setup_needed();
            if n != needs_setup() {
                needs_setup.set(n);
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    });
    rsx! {
        style { {FONT_CSS} }
        style { {CSS} }
        div { class: "root",
            // Tray + window-restore live at the app root so "Show HUSH" works on
            // every page (setup, eula, shell) — not just while Shell is mounted.
            if !running_blitz() {
                TrayHost {}
            }
            Titlebar {}
            div { class: "body",
                if !accepted() {
                    Eula { accepted }
                } else if needs_setup() {
                    SetupPage {}
                } else {
                    Shell {}
                }
            }
        }
    }
}

/// The post-onboarding app: owns the control mirror sync + page navigation.
#[component]
fn Shell() -> Element {
    let power = use_signal(|| true);
    let level = use_signal(|| 100u32);
    let notches = use_signal(Vec::<NotchParam>::new);
    let spectrum = use_signal(|| vec![0.0f32; SPECTRUM_BINS]);
    let spectrum_in = use_signal(|| vec![0.0f32; SPECTRUM_BINS]);
    let ctl = use_context_provider(|| Ctl {
        power,
        level,
        notches,
        spectrum,
        spectrum_in,
    });
    let page = use_signal(|| Page::Main);

    // Drive the local mirror from the UI; the IPC thread forwards it to hushd.
    use_effect(move || {
        if let Some(c) = CONTROLS.get() {
            c.set_intensity(if (ctl.power)() {
                (ctl.level)() as f32 / 100.0
            } else {
                0.0
            });
        }
    });
    use_effect(move || {
        if let Some(c) = CONTROLS.get() {
            c.set_notches((ctl.notches)());
        }
    });
    use_future(move || {
        let mut spectrum = ctl.spectrum;
        let mut spectrum_in = ctl.spectrum_in;
        async move {
            loop {
                if let Some(c) = CONTROLS.get() {
                    if let Ok(g) = c.spectrum.lock() {
                        spectrum.set(g.clone());
                    }
                    if let Ok(g) = c.spectrum_in.lock() {
                        spectrum_in.set(g.clone());
                    }
                }
                tokio::time::sleep(Duration::from_millis(45)).await;
            }
        }
    });

    rsx! {
        div { class: "screen",
            div { class: "pageview",
                match page() {
                    Page::Main => rsx! { MainPage {} },
                    Page::Freq => rsx! { FreqPage {} },
                    Page::Settings => rsx! { SettingsPage {} },
                }
            }
            nav { class: "nav",
                NavTab { page, target: Page::Main, icon: "⏻", label: "POWER" }
                NavTab { page, target: Page::Freq, icon: "≋", label: "BANDS" }
                NavTab { page, target: Page::Settings, icon: "⚙", label: "SETTINGS" }
            }
        }
    }
}

#[component]
fn NavTab(
    mut page: Signal<Page>,
    target: Page,
    icon: &'static str,
    label: &'static str,
) -> Element {
    let active = page() == target;
    rsx! {
        button {
            class: if active { "navtab on" } else { "navtab" },
            onclick: move |_| page.set(target),
            div { class: "navicon", "{icon}" }
            div { class: "navlabel", "{label}" }
        }
    }
}

// ---- app + tray icon, rendered from dist/hush.svg ----
const ICON_PNG_256: &[u8] = include_bytes!("../../../dist/hush-256.png");
const ICON_PNG_64: &[u8] = include_bytes!("../../../dist/hush-64.png");

/// Decode an 8-bit RGBA PNG → (width, height, rgba bytes).
fn decode_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = png::Decoder::new(bytes).read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

/// Window/taskbar icon for the webview window.
fn window_icon() -> Option<dioxus::desktop::tao::window::Icon> {
    let (w, h, rgba) = decode_rgba(ICON_PNG_256)?;
    dioxus::desktop::tao::window::Icon::from_rgba(rgba, w, h).ok()
}

/// Tray pixmaps (SNI wants ARGB32 network-byte-order); offer 64 + 256 so the host
/// scales from the closest size.
fn tray_icons() -> Vec<ksni::Icon> {
    [ICON_PNG_64, ICON_PNG_256]
        .iter()
        .filter_map(|b| {
            let (w, h, rgba) = decode_rgba(b)?;
            let mut data = Vec::with_capacity(rgba.len());
            for px in rgba.chunks_exact(4) {
                data.extend_from_slice(&[px[3], px[0], px[1], px[2]]); // RGBA -> ARGB
            }
            Some(ksni::Icon {
                width: w as i32,
                height: h as i32,
                data,
            })
        })
        .collect()
}

/// StatusNotifierItem tray via `ksni` (pure-Rust SNI). Unlike tray-icon's
/// libappindicator backend, both left-click (`activate`) and menu items deliver
/// events on Wayland/waybar. Every action just pings `SHOW_NOTIFY` / exits.
struct HushTray;
impl ksni::Tray for HushTray {
    fn id(&self) -> String {
        "hush".into()
    }
    fn title(&self) -> String {
        "HUSH".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_icons()
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        SHOW_NOTIFY.notify_one();
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show HUSH".into(),
                activate: Box::new(|_: &mut Self| SHOW_NOTIFY.notify_one()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit HUSH".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Webview-only, invisible: runs the tray and re-surfaces the window when the tray
/// (or a relaunch) pings `SHOW_NOTIFY`. The daemon is untouched — Quit closes only
/// the GUI.
#[component]
fn TrayHost() -> Element {
    use_hook(|| {
        ksni::TrayService::new(HushTray).spawn();
    });
    use_future(|| async move {
        loop {
            SHOW_NOTIFY.notified().await;
            let w = dioxus::desktop::window();
            w.window.set_minimized(false);
            w.window.set_visible(true);
            w.window.set_focus();
        }
    });
    rsx! {}
}

/// Custom frameless titlebar. Under webview it drives the real OS window
/// (drag/minimize/close); under Blitz there is no desktop window context, so
/// it renders a static bar and the compositor handles moves (e.g. Super+drag).
#[component]
fn Titlebar() -> Element {
    #[cfg(feature = "blitz")]
    if blitz_on() {
        return rsx! {
            div { class: "titlebar",
                div { class: "tbrand", "HUSH" span { class: "dot", "." } }
            }
        };
    }

    let window = dioxus::desktop::use_window();
    let (wd, wm, wc) = (window.clone(), window.clone(), window.clone());
    rsx! {
        div { class: "titlebar", onmousedown: move |_| wd.drag(),
            div { class: "tbrand", "HUSH" span { class: "dot", "." } }
            div { class: "wctl",
                button {
                    class: "wc",
                    onmousedown: move |e| e.stop_propagation(),
                    onclick: move |_| { wm.set_minimized(true); },
                    "–"
                }
                button {
                    class: "wc close",
                    onmousedown: move |e| e.stop_propagation(),
                    onclick: move |_| {
                        if CLOSE_TO_TRAY.load(Ordering::Relaxed) {
                            wc.window.set_visible(false);
                        } else {
                            wc.close();
                        }
                    },
                    "✕"
                }
            }
        }
    }
}

#[component]
fn Eula(accepted: Signal<bool>) -> Element {
    let mut acc = accepted;
    rsx! {
        div { class: "onb",
            div { class: "brand", "HUSH" span { class: "dot", "." } }
            div { class: "tagline", "AI MIC NOISE SUPPRESSION" }
            div { class: "license",
                p { class: "lhead", "BEFORE YOU START" }
                p { "HUSH runs NVIDIA's Maxine Audio Effects denoiser on your GPU to clean your microphone in real time, plus a built-in spectrum hum filter." }
                p { class: "lhead", "LICENSES" }
                p { "This software contains source code provided by NVIDIA Corporation. The NVIDIA Maxine runtime and models are used under the NVIDIA Software License Agreement and the NVIDIA Open Model License. HUSH is not affiliated with, sponsored by, or endorsed by NVIDIA." }
                p { "The application itself is provided \"as is\", without warranty of any kind. You are responsible for your use of it and for compliance with the above NVIDIA terms." }
                p { class: "lmut", "Requires an NVIDIA RTX GPU. Audio is processed entirely on-device — nothing leaves your machine." }
            }
            button {
                class: "agree",
                onclick: move |_| { persist_accept(); acc.set(true); },
                "I AGREE — START"
            }
        }
    }
}

/// WARP-style main page: one huge toggle + the suppression strength.
#[component]
fn MainPage() -> Element {
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
            div { class: "foot",
                span { class: "fdot" }
                "Select  HUSH  as your microphone in any app."
            }
        }
    }
}

const MAX_NOTCHES: usize = 8;
const FREQ_LO: f32 = 20.0;
const FREQ_HI: f32 = 1000.0;
const GAIN_BOT: f32 = -48.0; // deepest cut a band can dial in

/// Log frequency -> horizontal % across the 20 Hz..1 kHz decade.
fn fx(f: f32) -> f32 {
    let f = f.clamp(FREQ_LO, FREQ_HI);
    ((f / FREQ_LO).ln() / (FREQ_HI / FREQ_LO).ln()) * 100.0
}
fn x_to_freq(px: f32, w: f32) -> f32 {
    let t = (px / w.max(1.0)).clamp(0.0, 1.0);
    (FREQ_LO * (FREQ_HI / FREQ_LO).powf(t)).round()
}

/// Log-frequency slider position (0..1000) <-> Hz, so the fader travels evenly
/// across the audible decades instead of bunching the lows at the far left.
fn freq_to_pos(f: f32) -> f32 {
    fx(f) * 10.0
}
fn pos_to_freq(p: f32) -> f32 {
    x_to_freq(p, 1000.0)
}

/// 50/60 Hz preset: fundamental + 3 harmonics, deep narrow cuts.
fn set_mains(mut notches: Signal<Vec<NotchParam>>, base: f32) {
    notches.set(
        (1..=4)
            .map(|h| NotchParam {
                freq: base * h as f32,
                q: 18.0,
                gain: -30.0,
                enabled: true,
            })
            .collect(),
    );
}

/// Dedicated page: a parametric-EQ rack. A read-only response curve up top shows
/// the combined effect; each band below is a card with exact numeric + slider
/// control over frequency, width (Q) and cut depth, plus a per-band on/off.
#[component]
fn FreqPage() -> Element {
    let ctl = use_context::<Ctl>();
    let mut notches = ctl.notches;
    let spec = (ctl.spectrum)(); // with bands (denoised + notch)
    let spec_in = (ctl.spectrum_in)(); // without bands (denoised)
    let ns = notches();

    // --- with/without-bands spectrum; each band overlaid as a suppression zone ---
    let (vw, vh) = (1000.0f32, 176.0f32);
    let sx = |i: usize| fx((i as f32 + 1.0) * SPECTRUM_BIN_HZ) / 100.0 * vw;
    let sy = |m: f32| vh - m.clamp(0.0, 1.0) * (vh - 6.0);

    // WITH bands — filled area (the hero: what your bands leave behind)
    let mut sp = format!("M0,{:.0}", vh);
    for (i, &m) in spec.iter().enumerate() {
        sp.push_str(&format!(" L{:.1},{:.1}", sx(i), sy(m)));
    }
    sp.push_str(&format!(" L{:.0},{:.0} Z", vw, vh));

    // WITHOUT bands — open contour line; the gap down to the fill = what the bands remove
    let mut sp_in = String::new();
    for (i, &m) in spec_in.iter().enumerate() {
        sp_in.push_str(&format!(
            "{}{:.1},{:.1} ",
            if i == 0 { "M" } else { "L" },
            sx(i),
            sy(m)
        ));
    }
    let sp_in = sp_in.trim().to_string();

    // vertical octave guides
    let hz_grid: Vec<f32> = [50.0f32, 100.0, 200.0, 500.0]
        .iter()
        .map(|&f| fx(f) / 100.0 * vw)
        .collect();

    // a marker per band: (key, center%, label%, enabled, Hz). The real effect is the
    // measured gap between the two traces — no fake "range" box that contradicts it.
    let marks: Vec<(usize, f32, f32, bool, i32)> = ns
        .iter()
        .enumerate()
        .map(|(k, nt)| {
            let cx = fx(nt.freq);
            (k, cx, cx.clamp(4.0, 96.0), nt.enabled, nt.freq as i32)
        })
        .collect();

    let active = ns.iter().filter(|n| n.enabled && n.gain < -0.1).count();
    let full = ns.len() >= MAX_NOTCHES;

    rsx! {
        div { class: "page",
            div { class: "phead", "FREQUENCY BANDS" }
            div { class: "psub", "Dashed = your mic without bands; filled = with bands. The gap between them is what your cuts remove." }

            div { class: "card",
                div { class: "eq read",
                    svg { class: "eqsvg", view_box: "0 0 1000 176", preserve_aspect_ratio: "none",
                        for x in hz_grid.iter().copied() {
                            line { class: "vgrid", x1: "{x:.1}", y1: "0", x2: "{x:.1}", y2: "176" }
                        }
                        path { class: "sp", d: "{sp}" }
                        path { class: "spin", d: "{sp_in}" }
                    }
                    div { class: "eqlegend",
                        span { class: "lg in", "WITHOUT BANDS" }
                        span { class: "lg out", "WITH BANDS" }
                    }
                    for (k , cx , lx , en , hz) in marks.iter().copied() {
                        div { key: "{k}", class: "bandov",
                            div { class: if en { "zline" } else { "zline off" }, style: "left:{cx:.2}%" }
                            div { class: if en { "zlabel" } else { "zlabel off" }, style: "left:{lx:.2}%", "{hz}" }
                        }
                    }
                }
                div { class: "eqaxis",
                    for (f , lbl) in [(20.0f32, "20"), (50.0, "50"), (100.0, "100"), (250.0, "250"), (500.0, "500"), (1000.0, "1k")] {
                        span { style: "left:{fx(f)}%", "{lbl}" }
                    }
                }
                div { class: "eqread",
                    if ns.is_empty() {
                        "No bands yet — add one below, then slide it over a peak."
                    } else {
                        "{active} of {ns.len()} band(s) suppressing"
                    }
                }
            }

            div { class: "notchlist",
                for (k, nt) in ns.iter().enumerate() {
                    div {
                        key: "{k}",
                        class: if nt.enabled { "notch" } else { "notch off" },

                        div { class: "ntop",
                            span { class: "nfreq", "{nt.freq as i32} Hz" }
                            div { class: "ntools",
                                button {
                                    class: if nt.enabled { "nsw on" } else { "nsw" },
                                    title: if nt.enabled { "Mute this band" } else { "Enable this band" },
                                    onclick: move |_| {
                                        let mut v = notches();
                                        if k < v.len() { v[k].enabled = !v[k].enabled; notches.set(v); }
                                    },
                                    if nt.enabled { "ON" } else { "OFF" }
                                }
                                button {
                                    class: "nrm",
                                    title: "Remove band",
                                    onclick: move |_| {
                                        let mut v = notches();
                                        if k < v.len() { v.remove(k); notches.set(v); }
                                    },
                                    "✕"
                                }
                            }
                        }

                        BandSlider {
                            label: "FREQ", unit: "Hz",
                            value: nt.freq, min: FREQ_LO, max: FREQ_HI, step: 1.0, decimals: 0,
                            pct: freq_to_pos(nt.freq) / 10.0,
                            slider_min: 0.0, slider_max: 1000.0, slider_step: 1.0,
                            slider_value: freq_to_pos(nt.freq),
                            on_slide: move |p: f32| {
                                let mut v = notches();
                                if k < v.len() { v[k].freq = pos_to_freq(p); notches.set(v); }
                            },
                            on_type: move |f: f32| {
                                let mut v = notches();
                                if k < v.len() { v[k].freq = f.clamp(FREQ_LO, FREQ_HI).round(); notches.set(v); }
                            },
                        }
                        BandSlider {
                            label: "WIDTH", unit: "Q",
                            value: nt.q, min: 0.5, max: 36.0, step: 0.1, decimals: 1,
                            pct: (nt.q - 0.5) / 35.5 * 100.0,
                            slider_min: 0.5, slider_max: 36.0, slider_step: 0.1,
                            slider_value: nt.q,
                            on_slide: move |p: f32| {
                                let mut v = notches();
                                if k < v.len() { v[k].q = p; notches.set(v); }
                            },
                            on_type: move |q: f32| {
                                let mut v = notches();
                                if k < v.len() { v[k].q = q.clamp(0.5, 36.0); notches.set(v); }
                            },
                        }
                        BandSlider {
                            label: "DEPTH", unit: "dB",
                            value: nt.gain, min: GAIN_BOT, max: 0.0, step: 1.0, decimals: 0,
                            pct: (nt.gain - GAIN_BOT) / (0.0 - GAIN_BOT) * 100.0,
                            slider_min: GAIN_BOT, slider_max: 0.0, slider_step: 1.0,
                            slider_value: nt.gain,
                            on_slide: move |p: f32| {
                                let mut v = notches();
                                if k < v.len() { v[k].gain = p; notches.set(v); }
                            },
                            on_type: move |g: f32| {
                                let mut v = notches();
                                if k < v.len() { v[k].gain = g.clamp(GAIN_BOT, 0.0).round(); notches.set(v); }
                            },
                        }
                    }
                }
            }

            div { class: "quick",
                button {
                    class: "q add", disabled: full,
                    onclick: move |_| {
                        let mut v = notches();
                        if v.len() < MAX_NOTCHES {
                            v.push(NotchParam { freq: 120.0, q: 8.0, gain: -24.0, enabled: true });
                            notches.set(v);
                        }
                    },
                    if full { "Max 8 bands" } else { "+ Add band" }
                }
                button { class: "q", onclick: move |_| set_mains(notches, 50.0), "50 Hz mains" }
                button { class: "q", onclick: move |_| set_mains(notches, 60.0), "60 Hz mains" }
                button { class: "q", onclick: move |_| notches.set(Vec::new()), "Clear" }
            }
        }
    }
}

/// One labelled parameter row: a monospace number field the user can type into,
/// a filled slider, and a unit. Emits `on_type` on edit and `on_slide` on drag.
#[component]
fn BandSlider(
    label: &'static str,
    unit: &'static str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    decimals: u8,
    pct: f32,
    slider_min: f32,
    slider_max: f32,
    slider_step: f32,
    slider_value: f32,
    on_slide: EventHandler<f32>,
    on_type: EventHandler<f32>,
) -> Element {
    let shown = format!("{:.*}", decimals as usize, value);
    let pct = pct.clamp(0.0, 100.0);
    rsx! {
        div { class: "nrow",
            span { class: "nlab", "{label}" }
            input {
                class: "nnum",
                r#type: "number",
                min: "{min}", max: "{max}", step: "{step}",
                value: "{shown}",
                onchange: move |e| { if let Ok(v) = e.value().parse::<f32>() { on_type.call(v); } },
            }
            input {
                class: "mini",
                style: "--pct: {pct}%",
                r#type: "range",
                min: "{slider_min}", max: "{slider_max}", step: "{slider_step}",
                value: "{slider_value}",
                oninput: move |e| { if let Ok(v) = e.value().parse::<f32>() { on_slide.call(v); } },
            }
            span { class: "nunit", "{unit}" }
        }
    }
}

fn service_enabled() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "hush.service"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn unit_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/systemd/user/hush.service")
}

/// `LD_LIBRARY_PATH` for the daemon unit — the resolved SDK's lib dirs + host
/// driver. Empty if no SDK is installed yet (the unit still installs; the daemon
/// stays down until setup provisions the SDK, then the unit is regenerated).
fn sdk_lib_path() -> String {
    hush_core::sdk::ld_library_path().unwrap_or_default()
}

/// Write the systemd *user* unit (no root needed — it lives under `~/.config`),
/// pointing at the sibling `hushd` binary with the SDK on its library path.
fn install_unit() -> std::io::Result<()> {
    let hushd = std::env::current_exe()?.with_file_name("hushd");
    let unit = format!(
        "[Unit]\n\
         Description=HUSH denoiser daemon\n\
         After=pipewire.service wireplumber.service\n\
         Wants=pipewire.service\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={hushd}\n\
         Environment=LD_LIBRARY_PATH={libs}\n\
         Restart=on-failure\n\
         RestartSec=2\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        hushd = hushd.display(),
        libs = sdk_lib_path(),
    );
    let path = unit_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, unit)?;
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    Ok(())
}

/// Toggle launch-at-login. All `--user` scope — never needs sudo. Self-installs
/// the unit on first enable so the toggle works without a separate install step.
fn set_autostart(on: bool) {
    if on {
        let _ = install_unit();
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "enable", "hush.service"])
            .status();
    } else {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "hush.service"])
            .status();
    }
}

/// Ask the daemon to exit cleanly (its supervisor / the GUI reconnect loop brings
/// it back). Used when there's no systemd unit to `restart`.
fn send_shutdown() {
    use std::io::Write;
    if let Ok(mut s) = std::os::unix::net::UnixStream::connect(socket_path())
        && let Ok(mut buf) = serde_json::to_vec(&ClientMsg::Shutdown)
    {
        buf.push(b'\n');
        let _ = s.write_all(&buf);
    }
}

/// Restart the denoiser so it re-resolves the model / GPU. Prefers the systemd
/// user unit; if that isn't active (sibling-spawned daemon), asks it to exit so
/// `spawn_ipc_sync`'s reconnect loop respawns a fresh one.
fn restart_engine() {
    // Regenerate the unit so a freshly-provisioned/relocated SDK lands in the
    // service's LD_LIBRARY_PATH before it starts.
    if unit_path().exists() {
        let _ = install_unit();
    }
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "restart", "hush.service"])
        .status();
    let active = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "hush.service"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !active {
        send_shutdown();
    }
}

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

/// First-run / recovery page: pick the GPU, download its Maxine model, start up.
#[component]
fn SetupPage() -> Element {
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

/// Settings + QoL: engine status, launch-at-login, restart, about.
#[component]
fn SettingsPage() -> Element {
    let mut connected = use_signal(|| CONNECTED.load(Ordering::Relaxed));
    use_future(move || async move {
        loop {
            connected.set(CONNECTED.load(Ordering::Relaxed));
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    let mut autostart = use_signal(service_enabled);
    let mut to_tray = use_signal(|| CLOSE_TO_TRAY.load(Ordering::Relaxed));
    let gpus = use_hook(model::list_gpus);
    let mut gpu_label = use_signal({
        let gpus = gpus.clone();
        move || current_gpu_label(&gpus)
    });
    let multi_gpu = gpus.len() > 1;
    let conn = connected();
    rsx! {
        div { class: "page",
            div { class: "phead", "SETTINGS" }
            div { class: "card",
                div { class: "setrow",
                    div { class: "setmeta",
                        div { class: "sett", "Engine" }
                        div { class: "setd", "hushd denoiser service" }
                    }
                    div { class: if conn { "stat on" } else { "stat" },
                        span { class: "wdot" }
                        if conn { "CONNECTED" } else { "OFFLINE" }
                    }
                }
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

/// Vendored @font-face blocks with the woff2 bytes inlined as base64 data: URIs
/// at build time (build.rs) — no network needed to render the UI.
const FONT_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/fonts.css"));

const CSS: &str = r#"
:root{
  --bg:#070809; --card:#0e1217; --card2:#11161c; --line:#1b2128;
  --txt:#e9eef2; --mut:#5d6975; --acc:#37f2a6; --acc2:#0f7a52; --warn:#ff6b6b; --warn2:#6e2626;
}
*{ box-sizing:border-box; }
body{ margin:0; }
.root{
  font-family:'Chakra Petch', system-ui, sans-serif; color:var(--txt);
  height:100vh; padding:0; position:relative; overflow:hidden;
  display:flex; flex-direction:column;
  background:
    radial-gradient(130% 70% at 50% -8%, rgba(55,242,166,.12), transparent 58%),
    radial-gradient(90% 60% at 88% 112%, rgba(55,242,166,.05), transparent 60%),
    var(--bg);
}
.titlebar{ height:36px; display:flex; align-items:center; justify-content:space-between;
  padding:0 8px 0 18px; -webkit-user-select:none; user-select:none; position:relative; z-index:5; }
.tbrand{ font-family:'Chakra Petch'; font-weight:700; font-size:12px; letter-spacing:.30em; color:#8b97a2; }
.tbrand .dot{ color:var(--acc); text-shadow:0 0 8px var(--acc); }
.wctl{ display:flex; gap:2px; }
.wc{ width:30px; height:24px; border:none; background:transparent; color:#65717c; cursor:pointer;
  border-radius:6px; font-size:13px; line-height:1; transition:.12s; }
.wc:hover{ background:#1b2128; color:#dde4ea; }
.wc.close:hover{ background:#d0392b; color:#fff; }
.body{ flex:1; min-height:0; display:flex; }

/* ---- paged shell ---- */
.screen{ flex:1; min-height:0; width:100%; display:flex; flex-direction:column; }
.pageview{ flex:1; min-height:0; overflow-y:auto; overflow-x:hidden; padding:12px 22px 16px; }
.pageview::-webkit-scrollbar{ width:0; }
.page{ animation:rise .38s ease both; }
.phead{ font-family:'JetBrains Mono',monospace; font-size:11px; letter-spacing:.26em; color:var(--mut); margin:4px 0 5px; }
.psub{ font-size:12px; color:#8a949d; line-height:1.5; margin-bottom:14px; }

.nav{ display:flex; gap:4px; padding:8px 14px 12px; border-top:1px solid var(--line);
  background:linear-gradient(180deg, rgba(8,10,12,0), rgba(8,10,12,.55)); }
.navtab{ flex:1; display:flex; flex-direction:column; align-items:center; gap:4px; padding:8px 0;
  background:transparent; border:none; border-radius:12px; cursor:pointer; color:var(--mut); transition:.15s; }
.navtab:hover{ color:#aab4bd; background:#11161c; }
.navtab.on{ color:var(--acc); }
.navicon{ font-size:17px; line-height:1; }
.navtab.on .navicon{ text-shadow:0 0 10px var(--acc); }
.navlabel{ font-family:'JetBrains Mono',monospace; font-size:8.5px; letter-spacing:.16em; }

/* ---- WARP-style main toggle ---- */
.warpwrap{ display:flex; flex-direction:column; align-items:center; gap:16px; padding:26px 0 20px; }
.wbadge{ display:inline-flex; align-items:center; gap:7px; font-family:'JetBrains Mono',monospace;
  font-size:10px; letter-spacing:.24em; color:var(--mut); border:1px solid var(--line); border-radius:20px; padding:6px 14px; }
.wbadge.on{ color:var(--acc); border-color:rgba(55,242,166,.4); }
.wdot{ width:7px; height:7px; border-radius:50%; background:#39424b; flex:none; }
.wbadge.on .wdot, .stat.on .wdot{ background:var(--acc); box-shadow:0 0 8px var(--acc); animation:pulse 2s infinite; }
.warp{ width:230px; height:112px; border-radius:60px; border:1px solid var(--line); background:var(--card2);
  cursor:pointer; position:relative; display:block;
  transition:background .3s, border-color .3s, box-shadow .3s; }
.warp.on{ border-color:rgba(55,242,166,.5);
  background:linear-gradient(180deg, rgba(55,242,166,.18), rgba(55,242,166,.05));
  box-shadow:0 0 44px rgba(55,242,166,.28), inset 0 0 22px rgba(55,242,166,.07); }
.warp-knob{ position:absolute; top:7px; left:7px; width:96px; height:96px; border-radius:50%;
  background:#1b2128; color:var(--mut); display:flex; align-items:center; justify-content:center;
  transition:left .34s cubic-bezier(.34,1.4,.5,1), background .3s, color .3s, box-shadow .3s; }
/* geometrically-centered power glyph: currentColor box clipped by an svg mask */
.warp-knob::after{ content:""; width:44px; height:44px; background:currentColor;
  -webkit-mask:url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><g fill='none' stroke='white' stroke-width='2.2' stroke-linecap='round'><path d='M7.7 6.6 A6.6 6.6 0 1 0 16.3 6.6'/><path d='M12 3.2 L12 11.5'/></g></svg>") center/contain no-repeat;
  mask:url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><g fill='none' stroke='white' stroke-width='2.2' stroke-linecap='round'><path d='M7.7 6.6 A6.6 6.6 0 1 0 16.3 6.6'/><path d='M12 3.2 L12 11.5'/></g></svg>") center/contain no-repeat; }
.warp.on .warp-knob{ left:calc(100% - 7px - 96px);
  background:radial-gradient(circle at 35% 30%, #eafff5, var(--acc)); color:#04150d; box-shadow:0 0 26px var(--acc); }
/* toggled on but no engine attached — same slid-right position, red instead of green */
.wbadge.warn{ color:var(--warn); border-color:rgba(255,107,107,.45); }
.wbadge.warn .wdot{ background:var(--warn); box-shadow:0 0 8px var(--warn); animation:pulse 1.2s infinite; }
.warp.warn{ border-color:rgba(255,107,107,.55);
  background:linear-gradient(180deg, rgba(255,107,107,.16), rgba(255,107,107,.05));
  box-shadow:0 0 44px rgba(255,107,107,.26), inset 0 0 22px rgba(255,107,107,.07); }
.warp.warn .warp-knob{ left:calc(100% - 7px - 96px);
  background:radial-gradient(circle at 35% 30%, #ffe3e0, var(--warn)); color:#2a0d0d; box-shadow:0 0 26px var(--warn); }
.wtitle{ font-size:17px; font-weight:600; color:var(--txt); margin-top:2px; }
.wsub{ font-size:12px; color:#8a949d; text-align:center; line-height:1.5; max-width:300px; }
.mrow{ display:flex; justify-content:space-between; align-items:baseline; margin-bottom:12px; }
.mval{ font-family:'JetBrains Mono',monospace; font-weight:700; font-size:15px; color:var(--acc); }

/* ---- frequency bands ---- */
.bandlist{ display:flex; flex-direction:column; gap:7px; margin-top:14px; }
.band{ display:flex; align-items:center; justify-content:space-between; padding:10px 14px;
  border:1px solid var(--line); border-radius:12px; background:var(--card2); transition:.15s; }
.band.on{ border-color:rgba(55,242,166,.4); background:linear-gradient(180deg, rgba(55,242,166,.08), transparent); }
.bfreq{ font-family:'JetBrains Mono',monospace; font-weight:700; font-size:14px; color:var(--txt); }
.bnote{ font-family:'JetBrains Mono',monospace; font-size:9px; letter-spacing:.12em; color:var(--mut); margin-top:3px; }

/* ---- iOS-style switch ---- */
.switch{ width:46px; height:26px; border-radius:14px; border:1px solid var(--line); background:#12171d;
  cursor:pointer; padding:0; position:relative; transition:.18s; flex:none; }
.switch.on{ background:linear-gradient(180deg, var(--acc), var(--acc2)); border-color:transparent; }
.sknob{ position:absolute; top:2px; left:2px; width:20px; height:20px; border-radius:50%; background:#e9eef2;
  transition:transform .2s cubic-bezier(.34,1.4,.5,1); }
.switch.on .sknob{ transform:translateX(20px); background:#04150d; }

/* ---- parametric notches ---- */
.empty{ text-align:center; font-size:12px; color:var(--mut); padding:18px 0 6px; }
.notchlist{ display:flex; flex-direction:column; gap:9px; margin-top:14px; }
.notch{ border:1px solid var(--line); border-radius:13px; background:var(--card2); padding:12px 14px; }
.ntop{ display:flex; align-items:center; justify-content:space-between; margin-bottom:9px; }
.nfreq{ font-family:'JetBrains Mono',monospace; font-weight:700; font-size:16px; color:var(--acc); letter-spacing:.02em; }
.nrm{ width:24px; height:24px; border-radius:7px; border:1px solid var(--line); background:transparent;
  color:var(--mut); font-size:11px; cursor:pointer; transition:.13s; line-height:1; }
.nrm:hover{ background:#d0392b; color:#fff; border-color:transparent; }
.notch.off{ opacity:.5; }
.ntools{ display:flex; align-items:center; gap:7px; }
.nsw{ font-family:'JetBrains Mono',monospace; font-size:9px; letter-spacing:.12em; height:24px; min-width:40px;
  padding:0 9px; border-radius:7px; border:1px solid var(--line); background:transparent; color:var(--mut);
  cursor:pointer; transition:.13s; }
.nsw.on{ color:#04150d; background:var(--acc); border-color:transparent; box-shadow:0 0 10px rgba(55,242,166,.4); font-weight:700; }
.nsw:hover{ border-color:rgba(55,242,166,.5); }
.nsw.on:hover{ filter:brightness(1.08); }
.nrow{ display:flex; align-items:center; gap:10px; margin-top:8px; }
.nlab{ font-family:'JetBrains Mono',monospace; font-size:9px; letter-spacing:.18em; color:var(--mut); width:44px; flex:none; }
.nnum{ font-family:'JetBrains Mono',monospace; font-size:12px; color:var(--acc); width:56px; flex:none;
  text-align:right; background:#0c1116; border:1px solid var(--line); border-radius:7px; padding:5px 7px;
  outline:none; -moz-appearance:textfield; transition:border-color .13s; }
.nnum:focus{ border-color:var(--acc); }
.nnum::-webkit-inner-spin-button,.nnum::-webkit-outer-spin-button{ -webkit-appearance:none; margin:0; }
.nunit{ font-family:'JetBrains Mono',monospace; font-size:9px; letter-spacing:.1em; color:#3c454e; width:22px; flex:none; }
.mini{ -webkit-appearance:none; appearance:none; flex:1; height:5px; border-radius:4px; outline:none; cursor:pointer;
  background:linear-gradient(90deg, var(--acc) var(--pct), #1b2128 var(--pct)); }
.mini::-webkit-slider-thumb{ -webkit-appearance:none; width:15px; height:15px; border-radius:50%;
  background:#eafff5; border:2px solid var(--acc); box-shadow:0 0 8px rgba(55,242,166,.5); cursor:pointer; }

/* ---- live spectrum + suppression zones ---- */
.eq{ position:relative; height:176px; border:1px solid var(--line); border-radius:12px; overflow:hidden;
  background:radial-gradient(130% 100% at 50% 0%, rgba(55,242,166,.05), transparent 62%), #070a0d; }
.eqsvg{ position:absolute; inset:0; width:100%; height:100%; display:block; pointer-events:none; }
.vgrid{ stroke:rgba(150,170,185,.07); stroke-width:1; }
.sp{ fill:rgba(55,242,166,.18); stroke:var(--acc); stroke-width:1.6;
  filter:drop-shadow(0 0 4px rgba(55,242,166,.4)); }
.spin{ fill:none; stroke:rgba(200,212,222,.55); stroke-width:1.2; stroke-dasharray:4 3; }
.eqlegend{ position:absolute; top:8px; left:10px; display:flex; gap:12px; pointer-events:none; }
.eqlegend .lg{ font-family:'JetBrains Mono',monospace; font-size:8.5px; letter-spacing:.14em;
  display:flex; align-items:center; gap:5px; color:#7c8892; }
.eqlegend .lg::before{ content:""; width:12px; height:0; border-top:2px solid currentColor; }
.eqlegend .in{ color:#c8d4de; }
.eqlegend .in::before{ border-top-style:dashed; }
.eqlegend .out{ color:var(--acc); }
.bandov{ position:absolute; inset:0; pointer-events:none; }
.zline{ position:absolute; top:16px; bottom:0; width:0; border-left:1px dashed rgba(120,215,240,.5);
  transform:translateX(-.5px); }
.zline.off{ border-left:1px dashed rgba(150,170,185,.3); }
.zlabel{ position:absolute; top:3px; transform:translateX(-50%); font-family:'JetBrains Mono',monospace;
  font-size:9px; font-weight:700; color:#bfe8f4; letter-spacing:.02em; white-space:nowrap;
  text-shadow:0 0 6px rgba(0,0,0,.85); }
.zlabel.off{ color:#5a656e; font-weight:500; }
.eqaxis{ position:relative; height:13px; margin-top:7px; }
.eqaxis span{ position:absolute; transform:translateX(-50%); font-family:'JetBrains Mono',monospace;
  font-size:8.5px; color:#3c454e; }
.eqread{ font-family:'JetBrains Mono',monospace; font-size:10.5px; letter-spacing:.06em; color:var(--mut);
  text-align:center; margin-top:11px; min-height:14px; }

/* ---- settings ---- */
.setrow{ display:flex; align-items:center; justify-content:space-between; gap:12px; padding:13px 0;
  border-bottom:1px solid rgba(27,33,40,.55); }
.setrow:last-child{ border-bottom:none; padding-bottom:2px; }
.setrow:first-child{ padding-top:2px; }
.sett{ font-size:14px; color:var(--txt); font-weight:500; }
.setd{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.03em; color:var(--mut); margin-top:4px; }
.stat{ display:inline-flex; align-items:center; gap:6px; font-family:'JetBrains Mono',monospace;
  font-size:10px; letter-spacing:.16em; color:var(--mut); }
.stat.on{ color:var(--acc); }
.sbtn{ font-family:'Chakra Petch'; font-size:11px; letter-spacing:.12em; color:var(--txt);
  background:var(--card2); border:1px solid var(--line); border-radius:9px; padding:9px 15px; cursor:pointer; transition:.15s; }
.sbtn:hover{ border-color:rgba(55,242,166,.45); color:var(--acc); }
.setabout{ color:#b9c2cc; font-size:12px; line-height:1.65; margin:9px 0 11px; }

/* ---- setup / GPU picker + download ---- */
.gpurow{ width:100%; display:flex; align-items:center; justify-content:space-between; gap:12px;
  margin-top:9px; padding:12px 14px; border:1px solid var(--line); border-radius:12px;
  background:var(--card2); color:var(--txt); cursor:pointer; text-align:left; transition:.14s; }
.gpurow:hover:not(:disabled){ border-color:rgba(55,242,166,.35); }
.gpurow.on{ border-color:rgba(55,242,166,.55); background:linear-gradient(180deg, rgba(55,242,166,.08), rgba(55,242,166,.02)); }
.gpurow:disabled{ opacity:.55; cursor:default; }
.gpuname{ font-size:13px; font-weight:600; color:var(--txt); }
.gpuarch{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.06em; color:var(--mut); margin-top:3px; }
.gpudot{ width:16px; height:16px; border-radius:50%; border:2px solid var(--line); flex:none; transition:.14s; }
.gpudot.on{ border-color:var(--acc); background:radial-gradient(circle at 50% 50%, var(--acc) 42%, transparent 46%);
  box-shadow:0 0 10px rgba(55,242,166,.5); }
.dlbtn{ width:100%; margin-top:14px; font-family:'Chakra Petch'; font-weight:600; font-size:14px; letter-spacing:.04em;
  color:#04150d; background:linear-gradient(180deg, var(--acc), var(--acc2)); border:none; border-radius:12px;
  padding:14px; cursor:pointer; box-shadow:0 0 22px rgba(55,242,166,.3); transition:.15s; }
.dlbtn:hover{ filter:brightness(1.06); }
.dlbar{ margin-top:14px; height:10px; border-radius:6px; background:#12171d; overflow:hidden; border:1px solid var(--line); }
.dlfill{ height:100%; background:linear-gradient(90deg, var(--acc2), var(--acc));
  box-shadow:0 0 12px rgba(55,242,166,.5); transition:width .15s linear; }
.dlnote{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.04em; color:var(--mut);
  margin-top:9px; text-align:center; }
.dlerr{ font-family:'JetBrains Mono',monospace; font-size:10.5px; line-height:1.5; color:var(--warn);
  background:rgba(255,107,107,.07); border:1px solid rgba(255,107,107,.25); border-radius:9px; padding:10px 12px; margin-top:11px; }
.root::before{ content:""; position:fixed; inset:0; pointer-events:none; opacity:.04;
  background-image:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='120' height='120'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.9' numOctaves='2'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E"); }

.brand{ font-weight:700; letter-spacing:.34em; text-transform:uppercase; }
.brand.sm{ font-size:15px; letter-spacing:.30em; }
.brand .dot{ color:var(--acc); text-shadow:0 0 10px var(--acc); }

/* ---- onboarding ---- */
.onb{ max-width:520px; margin:0 auto; animation:rise .5s ease both; }
.onb .brand{ font-size:30px; text-align:center; margin-top:8px; }
.tagline{ text-align:center; font-family:'JetBrains Mono',monospace; font-size:11px;
  letter-spacing:.34em; color:var(--mut); margin:8px 0 22px; }
.license{ background:var(--card); border:1px solid var(--line); border-radius:14px;
  padding:18px 20px; max-height:46vh; overflow:auto; }
.license p{ color:#b9c2cc; font-size:13px; line-height:1.6; margin:0 0 12px; }
.lhead{ color:var(--acc) !important; font-family:'JetBrains Mono',monospace; font-size:10px;
  letter-spacing:.2em; margin-top:6px !important; }
.lmut{ color:var(--mut) !important; font-size:11.5px !important; }
.agree{ width:100%; margin-top:18px; border:none; border-radius:12px; cursor:pointer;
  background:linear-gradient(180deg,var(--acc),var(--acc2)); color:#04150d; font-weight:700;
  font-family:'Chakra Petch'; letter-spacing:.16em; font-size:14px; padding:14px;
  box-shadow:0 0 24px rgba(55,242,166,.35); transition:transform .12s, box-shadow .2s; }
.agree:hover{ transform:translateY(-1px); box-shadow:0 0 34px rgba(55,242,166,.55); }

/* ---- deck ---- */
.deck{ max-width:440px; margin:0 auto; animation:rise .45s ease both; }
.top{ display:flex; justify-content:space-between; align-items:center; margin-bottom:16px; }
.sub{ font-family:'JetBrains Mono',monospace; font-size:9.5px; letter-spacing:.22em;
  color:var(--mut); margin-top:5px; }
.pill{ display:flex; align-items:center; gap:7px; font-family:'JetBrains Mono',monospace;
  font-size:10px; letter-spacing:.18em; color:var(--mut); background:var(--card);
  border:1px solid var(--line); border-radius:20px; padding:6px 12px; }
.pill.on{ color:var(--acc); border-color:rgba(55,242,166,.4); }
.led{ width:7px; height:7px; border-radius:50%; background:#39424b; }
.pill.on .led{ background:var(--acc); box-shadow:0 0 8px var(--acc); animation:pulse 2s infinite; }

.card{ background:linear-gradient(180deg,var(--card),var(--card2)); border:1px solid var(--line);
  border-radius:16px; padding:18px; margin-bottom:14px; }
.cardtop{ display:flex; justify-content:space-between; align-items:center; }
.label{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.2em; color:var(--mut); }
.label2{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.12em;
  color:var(--mut); margin:18px 0 10px; }
.pwr{ width:34px; height:34px; border-radius:10px; border:1px solid var(--line);
  background:var(--card2); color:var(--mut); font-size:15px; cursor:pointer; transition:.15s; }
.pwr.on{ color:var(--acc); border-color:rgba(55,242,166,.45); box-shadow:0 0 16px rgba(55,242,166,.28); }

.meter{ display:flex; align-items:baseline; gap:4px; margin:14px 0 16px; }
.big{ font-family:'JetBrains Mono',monospace; font-weight:700; font-size:54px; line-height:1;
  color:var(--txt); text-shadow:0 0 22px rgba(55,242,166,.25); font-variant-numeric:tabular-nums; }
.unit{ font-family:'JetBrains Mono',monospace; font-size:20px; color:var(--mut); }

.fader{ -webkit-appearance:none; appearance:none; width:100%; height:8px; border-radius:6px;
  background:linear-gradient(90deg, var(--acc) var(--pct), #1b2128 var(--pct)); outline:none; }
.fader:disabled{ filter:grayscale(.8) brightness(.7); }
.fader::-webkit-slider-thumb{ -webkit-appearance:none; width:20px; height:20px; border-radius:50%;
  background:#eafff5; border:3px solid var(--acc); box-shadow:0 0 12px var(--acc); cursor:pointer; }
.scale{ display:flex; justify-content:space-between; font-family:'JetBrains Mono',monospace;
  font-size:9px; letter-spacing:.18em; color:#3c454e; margin-top:9px; }

.spec{ display:flex; align-items:flex-end; gap:1px; height:96px; padding:8px 4px 0;
  background:radial-gradient(120% 100% at 50% 100%, rgba(55,242,166,.06), transparent 70%), #090c10;
  border:1px solid var(--line); border-radius:10px; }
.spec.tall{ height:132px; }
.bar{ flex:1; min-width:0; border-radius:2px 2px 0 0; cursor:pointer;
  background:linear-gradient(180deg, var(--acc), var(--acc2));
  box-shadow:0 0 6px rgba(55,242,166,.45); transition:height .06s linear; }
.bar.n{ background:linear-gradient(180deg, var(--warn), var(--warn2)); box-shadow:0 0 6px rgba(255,107,107,.5); }
.axis{ display:flex; justify-content:space-between; font-family:'JetBrains Mono',monospace;
  font-size:8.5px; color:#39424b; padding:5px 4px 0; }

.chips{ display:flex; flex-wrap:wrap; gap:6px; }
.chip{ font-family:'JetBrains Mono',monospace; font-size:11px; border:1px solid var(--line);
  background:var(--card2); color:#9aa6b1; border-radius:9px; padding:6px 11px; cursor:pointer; transition:.13s; }
.chip:hover{ border-color:#33414a; color:#cdd6df; }
.chip.on{ background:var(--warn); border-color:var(--warn); color:#260606; font-weight:700;
  box-shadow:0 0 12px rgba(255,107,107,.4); }
.quick{ display:flex; gap:8px; margin-top:11px; }
.q{ flex:1; font-family:'Chakra Petch'; letter-spacing:.06em; font-size:12px; border:1px solid var(--line);
  background:var(--card2); color:#c3ccd4; border-radius:9px; padding:9px; cursor:pointer; transition:.13s; }
.q:hover{ border-color:rgba(55,242,166,.4); color:var(--acc); }
.q.add{ flex:1.4; border-color:rgba(55,242,166,.35); color:var(--acc); }
.q.add:hover{ background:rgba(55,242,166,.1); }
.q:disabled{ opacity:.4; cursor:default; color:var(--mut); border-color:var(--line); background:var(--card2); }

.foot{ display:flex; align-items:center; gap:9px; font-family:'JetBrains Mono',monospace;
  font-size:10.5px; letter-spacing:.05em; color:var(--mut); margin-top:6px; padding:2px 4px; }
.fdot{ width:6px; height:6px; border-radius:50%; background:var(--acc); box-shadow:0 0 8px var(--acc); }
.foot{ } .foot :where(span):not(.fdot){ color:var(--acc); }

@keyframes rise{ from{ opacity:0; transform:translateY(10px);} to{ opacity:1; transform:none; } }
@keyframes pulse{ 0%,100%{ opacity:1;} 50%{ opacity:.35; } }
"#;

#[allow(dead_code)]
fn blitz_on() -> bool {
    std::env::var_os("NV_MAXINE_BLITZ").is_some_and(|v| v == "1")
}

/// Background thread that keeps the GUI's `CONTROLS` mirror in sync with `hushd`:
/// pushes local intensity/notch changes to the daemon and pulls its spectrum back.
/// Reconnects on its own, so the daemon and GUI lifecycles are fully independent.
fn spawn_ipc_sync(mirror: std::sync::Arc<Controls>) {
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

/// Drive one daemon connection until it drops: send control deltas, receive frames.
async fn run_sync(stream: UnixStream, mirror: &Controls) -> std::io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut lines = BufReader::new(rd).lines();
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    // Force an initial push so the daemon adopts this GUI's current settings.
    let mut last_intensity = f32::NAN;
    let mut last_gen = u64::MAX;

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

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Hold an exclusive `flock` on a runtime lockfile for the process lifetime, so a
/// second GUI launch bails out. The lock releases automatically on exit (even crash).
fn acquire_single_instance() -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false) // it's a lock file — content is irrelevant, never truncate
        .write(true)
        .open(runtime_dir().join("hush-gui.lock"))
        .ok()?;
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (rc == 0).then_some(f)
}

fn gui_sock_path() -> PathBuf {
    runtime_dir().join("hush-gui.sock")
}

/// Best-effort: ask an already-running GUI to surface its window. Works on any DE,
/// so "close to tray" never strands the window even without a tray host.
fn notify_show_if_running() {
    use std::io::Write;
    if let Ok(mut s) = std::os::unix::net::UnixStream::connect(gui_sock_path()) {
        let _ = s.write_all(b"show\n");
    }
}

/// Listen for "show" pokes from later launches; flip `SHOW_REQUESTED` for the UI.
fn spawn_show_listener() {
    let path = gui_sock_path();
    let _ = std::fs::remove_file(&path);
    if let Ok(listener) = std::os::unix::net::UnixListener::bind(&path) {
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if conn.is_ok() {
                    SHOW_NOTIFY.notify_one();
                }
            }
        });
    }
}

fn main() {
    // Single instance: poke any running GUI to surface, then bail if one holds the lock.
    notify_show_if_running();
    let _lock = match acquire_single_instance() {
        Some(f) => f,
        None => {
            eprintln!("HUSH is already running — raising the existing window.");
            return;
        }
    };
    spawn_show_listener();
    CLOSE_TO_TRAY.store(load_close_to_tray(), std::sync::atomic::Ordering::Relaxed);

    // `CONTROLS` is now a local *mirror* of the daemon's state, not the engine's:
    // the UI reads/writes it exactly as before, and a background thread syncs it to
    // `hushd` over the control socket. Closing the GUI just drops that socket.
    let controls = Controls::new();
    let _ = CONTROLS.set(controls.clone());
    spawn_ipc_sync(controls);

    #[cfg(feature = "blitz")]
    if blitz_on() {
        let attrs = dioxus_native::WindowAttributes::default()
            .with_title("HUSH")
            .with_decorations(false)
            .with_surface_size(dioxus_native::LogicalSize::new(480.0, 720.0));
        dioxus_native::launch_cfg(
            App,
            vec![],
            vec![Box::new(
                dioxus_native::Config::new().with_window_attributes(attrs),
            )],
        );
        return;
    }

    use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
    let win = WindowBuilder::new()
        .with_title("HUSH")
        .with_inner_size(LogicalSize::new(480.0, 720.0))
        .with_decorations(false)
        .with_resizable(false)
        .with_window_icon(window_icon());
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(win))
        .launch(App);
}
