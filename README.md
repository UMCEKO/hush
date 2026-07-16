<p align="center">
  <img src="dist/hush.svg" width="96" alt="HUSH logo">
</p>

<h1 align="center">HUSH</h1>

<p align="center">
  <b>NVIDIA Broadcast-style AI noise removal for Linux.</b><br>
  Runs NVIDIA's Maxine denoiser on your GPU and exposes the cleaned signal as a
  virtual PipeWire microphone — select <b>"HUSH"</b> in Discord, OBS, Zoom, anything.
</p>

---

NVIDIA never brought Broadcast / RTX Voice to Linux. HUSH runs the same Maxine
Audio Effects denoiser models in real time on your RTX card: keyboard clatter,
fans, and background noise are stripped from your mic while your voice passes
through untouched. A parametric band section additionally notches out mains hum
and other tonal noise.

- **Native** — Rust engine on PipeWire, small always-on daemon, GUI with tray icon
- **Works everywhere** — apps just see a normal microphone named "HUSH"
- **One toggle** — denoise strength is adjustable live

## Requirements

- Linux x86_64 with **PipeWire**
- **NVIDIA GPU, Turing or newer** (GTX 16 / RTX 20-series and up) with the proprietary driver
- The Maxine runtime and per-GPU models are **downloaded on first launch** (they are not bundled)

## Install

### Flatpak

```sh
flatpak remote-add --if-not-exists hush https://umceko.github.io/hush/index.flatpakrepo
flatpak install hush io.github.umceko.hush
```

### Arch (AUR)

```sh
yay -S hush-mic-bin   # prebuilt release binaries
yay -S hush-mic       # or build from source
```

### NixOS / Nix

Flake with a Home Manager module:

```nix
# flake input
inputs.hush.url = "github:UMCEKO/hush";

# home-manager
imports = [ inputs.hush.homeManagerModules.default ];
services.hush.enable = true;
```

### Binaries

Prebuilt `hush` (GUI) and `hushd` (daemon) are attached to every
[GitHub release](https://github.com/UMCEKO/hush/releases/latest).

## Usage

1. Launch **HUSH**. On first run it downloads the Maxine runtime and the model
   for your GPU.
2. Pick your real microphone, toggle the denoiser, set the strength.
3. In any app, select the microphone named **"HUSH"**.

The engine runs in `hushd` (a systemd user service where available — the GUI
starts it automatically), so denoising keeps working after you close the window.

## How it works

```
real mic ──► hushd: Maxine AFX denoiser (GPU) ──► hum notch filters ──► "HUSH" virtual source
```

`hushd` captures your input, runs it through NVIDIA's Maxine Audio Effects SDK,
and publishes the result as a PipeWire `Audio/Source/Virtual` node. The GUI
(`hush`) is a thin control panel over a local socket.

## Building from source

```sh
# native deps: pipewire dbus webkitgtk-4.1 gtk3 libsoup3 openssl zstd (see flake.nix)
cargo build --release -p hush-app -p hush-engine
```

Linking needs the NVIDIA AFX link libraries; point `NVAFX_LINK_DIR` at an
extracted [afx-link tarball](https://cdn.hush.umceko.com/sdk/2.1.0/afx-link-x86_64.tar.zst)
(`nvafx/lib` + CUDA runtime). A Nix dev shell is provided: `nix develop`.

## License

[MIT](LICENSE). The NVIDIA Maxine runtime is downloaded under NVIDIA's own
terms — see [NVIDIA_NOTICE](dist/NVIDIA_NOTICE.txt). HUSH is not affiliated
with, sponsored by, or endorsed by NVIDIA.
