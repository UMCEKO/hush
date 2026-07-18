//! First-run license screen.

use dioxus::prelude::*;

use crate::state::persist_accept;

#[component]
pub(crate) fn Eula(accepted: Signal<bool>) -> Element {
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
