# Contributing to HUSH

Thanks for your interest! Here's the lay of the land.

## Crate map

| Crate | What it is |
|---|---|
| `hush-core` | GPU-free plumbing: IPC wire types, shared `Controls`, SDK/model download + verification. No NVIDIA or GUI deps — keep it that way. |
| `hush-engine` | The Maxine FFI (`lib.rs`), the PipeWire audio engine (`engine.rs`), and the `hushd` daemon binary. Links `libnv_audiofx` at build time. |
| `hush-app` | The Dioxus (WebKitGTK) GUI, binary `hush`. `main.rs` is an entry point **only** — pages live in `src/components/`, cross-thread state in `src/state.rs`, daemon sync in `src/ipc.rs`, systemd handling in `src/service.rs`, window/tray plumbing in `src/window.rs` + `src/tray.rs`, CSS in `src/style.rs`. Please keep it that way in PRs. |

The GUI and daemon are separate processes talking newline-delimited JSON over
`$XDG_RUNTIME_DIR/hush.sock`. Protocol evolution rule: every new `StateFrame`
field gets `#[serde(default)]`, unknown commands are ignored — old GUIs must
keep working against new daemons and vice versa.

## Building

Linking needs the NVIDIA AFX link libraries (not the full SDK):

```sh
curl -LO https://cdn.hush.umceko.com/sdk/2.1.0/afx-link-x86_64.tar.zst
mkdir afx-link && bsdtar -xf afx-link-x86_64.tar.zst -C afx-link
NVAFX_LINK_DIR=$PWD/afx-link cargo build --release -p hush-app -p hush-engine
```

Native deps: `pipewire dbus webkit2gtk-4.1 gtk3 libsoup3 openssl zstd` (see
`flake.nix`). On Nix, `nix develop` provides everything.

At runtime the daemon needs the Maxine runtime + a per-GPU model — the app's
setup page downloads both on first launch.

## Testing a change

There's no test suite for the audio path (yet — contributions welcome); verify
by running:

1. `cargo build --release -p hush-app -p hush-engine`
2. Start/restart the daemon (`systemctl --user restart hush.service`, or let the
   GUI spawn it) and click through: main toggle, suppression slider, mic picker,
   bands page, settings, tray hide/show.
3. `cargo fmt` and `cargo clippy` before pushing.

## Gotchas worth knowing

- **Never drive window show/hide through dioxus state or futures** — while the
  window is hidden WebKit stops flushing, which stalls the vdom poll loop. Use
  `window::set_window_visible_from_any_thread` (see its comment).
- The virtual mic must be `media.class=Audio/Source/Virtual` — plain
  `Audio/Source` comes up portless (see `engine.rs`).
- The dioxus dependency is a pinned fork (see `[patch.crates-io]` in the root
  `Cargo.toml`); bumping it is a deliberate, tested change — don't drive-by
  update it.

## Releases

Maintainer-only: push to the `release` branch with a bumped workspace version;
CI tags, builds, and publishes to GitHub Releases, Flatpak (gh-pages), and AUR.
