{ lib
, stdenv
, rustPlatform
, fetchurl
, runCommandLocal
, pkg-config
, wrapGAppsHook3
, patchelf
, zstd
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
}:

let
  # Link-time NVIDIA libs (libnv_audiofx + libcudart) from the HUSH CDN. Only the
  # daemon links these; the runtime CUDA/TensorRT stack + models are downloaded by
  # the app at first launch, so the store package carries no heavy NVIDIA payload.
  afxLink = fetchurl {
    url = "https://cdn.hush.umceko.com/sdk/2.1.0/afx-link-x86_64.tar.zst";
    hash = "sha256-6zgRNNz654o1uF8GxtLIcQPvu35zbg3LzT2qT8NZ66U=";
  };
  afxLinkDir = runCommandLocal "afx-link" { } ''
    mkdir -p $out
    tar --use-compress-program=${zstd}/bin/zstd -xf ${afxLink} -C $out
  '';
in
rustPlatform.buildRustPackage {
  pname = "hush";
  version = "1.0.0";

  src = lib.cleanSource ../.;

  # dioxus/blitz are git deps pinned by full SHA in Cargo.lock; one FOD per repo.
  cargoLock = {
    lockFile = ../Cargo.lock;
    outputHashes = {
      "dioxus-0.8.0-alpha.0" = "sha256-4f4I7FeAGrt20WHY9NH9pJTXuLeB5fYYe6O+m7c4ZOY=";
      "blitz-dom-0.3.0-alpha.5" = "sha256-sygNvgWcwGuntaoMULKdBk1Ran9hCPIOfFzXJMfXJ6Q=";
    };
  };

  # Point build.rs at the CDN link libs (no SDK on the build machine).
  env.NVAFX_LINK_DIR = "${afxLinkDir}";

  # Only the shippable binaries (skip the dev tools denoise/mic/inttest).
  cargoBuildFlags = [ "-p" "hush-app" "-p" "hush-engine" "--bin" "hush" "--bin" "hushd" ];
  doCheck = false; # engine tests need a GPU; core tests hit the network

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

  postInstall = ''
    install -Dm644 dist/io.github.umceko.hush.desktop $out/share/applications/io.github.umceko.hush.desktop
    install -Dm644 dist/io.github.umceko.hush.metainfo.xml $out/share/metainfo/io.github.umceko.hush.metainfo.xml
    install -Dm644 dist/hush.svg $out/share/icons/hicolor/scalable/apps/io.github.umceko.hush.svg
  '';

  preFixup = ''
    gappsWrapperArgs+=(
      --prefix PATH : ${lib.makeBinPath [ pulseaudio ]}
    )
  '';

  # hushd dlopens the CUDA/TensorRT stack + denoiser feature lib from the SDK the
  # app provisions at runtime (found via LD_LIBRARY_PATH), so no SDK rpath is baked.
  # It still needs libstdc++ (for the NVIDIA blobs) and the host driver's libcuda.
  dontPatchELF = true;
  postFixup = ''
    for f in "$out/bin/hushd" "$out/bin/.hushd-wrapped"; do
      if [ -f "$f" ] && isELF "$f"; then
        patchelf --add-rpath "${lib.getLib stdenv.cc.cc}/lib:/run/opengl-driver/lib" "$f"
      fi
    done
  '';

  meta = {
    description = "NVIDIA Maxine denoiser daemon + GUI exposing a clean virtual microphone via PipeWire";
    homepage = "https://github.com/UMCEKO/hush";
    platforms = [ "x86_64-linux" ];
    license = lib.licenses.unfree; # bundles NVIDIA link libs
    mainProgram = "hush";
  };
}
