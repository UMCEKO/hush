//! App + tray icon (rendered from the crate's PNG assets) and the ksni
//! StatusNotifierItem implementation.

use crate::window::set_window_visible_from_any_thread;

const ICON_PNG_256: &[u8] = include_bytes!("../assets/hush-256.png");
const ICON_PNG_64: &[u8] = include_bytes!("../assets/hush-64.png");

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
pub(crate) fn window_icon() -> Option<dioxus::desktop::tao::window::Icon> {
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
pub(crate) struct HushTray;
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
        set_window_visible_from_any_thread(true);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show HUSH".into(),
                activate: Box::new(|_: &mut Self| set_window_visible_from_any_thread(true)),
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
