{ lib
, stdenv
, rustPlatform
, pkg-config
, wrapGAppsHook3
, patchelf
, pipewire
, dbus
, webkitgtk_4_1
, gtk3
, libsoup_3
, glib
, glib-networking
, openssl
, xdotool
, pulseaudio # pactl, used by the engine to manage the virtual source
, dioxus-src
, blitz-src
, nvafx-sdk
}:

rustPlatform.buildRustPackage {
  pname = "hush";
  version = "0.1.0";

  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;

  # The Cargo.toml pins dioxus/blitz to checkouts in $HOME; point them at the
  # flake inputs so the sandboxed build can see them.
  postPatch = ''
    substituteInPlace Cargo.toml \
      --replace-fail "/home/umceko/dioxus" "${dioxus-src}" \
      --replace-fail "/home/umceko/blitz" "${blitz-src}"
  '';

  nativeBuildInputs = [
    pkg-config
    wrapGAppsHook3
    rustPlatform.bindgenHook # pipewire-sys / libspa-sys
  ];

  buildInputs = [
    pipewire
    dbus # ksni tray
    webkitgtk_4_1
    gtk3
    libsoup_3
    glib
    glib-networking
    openssl
    xdotool # libxdo for dioxus-desktop
  ];

  # build.rs reads this to find libnv_audiofx + the SDK's CUDA/TensorRT libs.
  env.NVAFX_SDK = "${nvafx-sdk}";

  postInstall = ''
    mv $out/bin/app $out/bin/hush
    rm -f $out/bin/denoise $out/bin/mic $out/bin/inttest

    install -Dm644 dist/hush.desktop $out/share/applications/hush.desktop
    install -Dm644 dist/hush.svg $out/share/icons/hicolor/scalable/apps/hush.svg
  '';

  # No NVAFX_MODEL default: the app detects the GPU and resolves/downloads the
  # matching model at setup (a hard-pinned sm_89 path would defeat that).
  preFixup = ''
    gappsWrapperArgs+=(
      --prefix PATH : ${lib.makeBinPath [ pulseaudio ]}
    )
  '';

  # Keep build.rs's DT_RPATH intact (the SDK dlopens its feature lib through the
  # executable's RPATH), skip the rpath shrinker, and append what build.rs can't
  # know about: libstdc++ for the NVIDIA blobs and the host driver's libcuda.
  dontPatchELF = true;
  postFixup = ''
    for f in "$out"/bin/* "$out"/bin/.*-wrapped; do
      if [ -f "$f" ] && isELF "$f"; then
        patchelf --force-rpath --add-rpath \
          "${nvafx-sdk}/features/denoiser/lib:${lib.getLib stdenv.cc.cc}/lib:/run/opengl-driver/lib" \
          "$f"
      fi
    done
  '';

  meta = {
    description = "NVIDIA Maxine denoiser daemon + GUI exposing a clean virtual microphone via PipeWire";
    platforms = [ "x86_64-linux" ];
    license = lib.licenses.unfree; # links the proprietary Maxine AFX SDK
    mainProgram = "hush";
  };
}
