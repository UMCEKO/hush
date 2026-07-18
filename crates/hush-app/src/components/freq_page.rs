//! "BANDS" page: the parametric-EQ rack over the live spectrum.

use dioxus::prelude::*;
use hush_core::{NotchParam, SPECTRUM_BIN_HZ};

use crate::state::Ctl;

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
pub(crate) fn FreqPage() -> Element {
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
