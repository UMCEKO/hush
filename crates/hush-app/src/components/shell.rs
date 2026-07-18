//! App root + navigation: onboarding/setup routing, the paged shell, titlebar,
//! and the tray host.

use std::sync::atomic::Ordering;
use std::time::Duration;

use dioxus::prelude::*;
use hush_core::{NotchParam, SPECTRUM_BINS};

use super::eula::Eula;
use super::freq_page::FreqPage;
use super::main_page::MainPage;
use super::settings_page::SettingsPage;
use super::setup_page::SetupPage;
use crate::state::{CLOSE_TO_TRAY, CONTROLS, Ctl, eula_accepted, running_blitz, setup_needed};
use crate::style::{CSS, FONT_CSS};
use crate::tray::HushTray;
use crate::window::GTK_WINDOW;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Page {
    Main,
    Freq,
    Settings,
}

#[component]
pub(crate) fn App() -> Element {
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

/// Webview-only, invisible: runs the tray and re-surfaces the window when the tray
/// (or a relaunch) pings `SHOW_NOTIFY`. The daemon is untouched — Quit closes only
/// the GUI.
#[component]
fn TrayHost() -> Element {
    use_hook(|| {
        ksni::TrayService::new(HushTray).spawn();
    });
    use_hook(|| {
        use dioxus::desktop::tao::platform::unix::WindowExtUnix;
        let gw = dioxus::desktop::window().window.gtk_window().clone();
        GTK_WINDOW.with(|w| *w.borrow_mut() = Some(gw));
    });
    rsx! {}
}

/// Custom frameless titlebar. Under webview it drives the real OS window
/// (drag/minimize/close); under Blitz there is no desktop window context, so
/// it renders a static bar and the compositor handles moves (e.g. Super+drag).
#[component]
fn Titlebar() -> Element {
    #[cfg(feature = "blitz")]
    if crate::state::blitz_on() {
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
