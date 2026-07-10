{ lib
, stdenv
, craneLib
, rustPlatform
, llvmPackages
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
  pname = "hush";
  version = "0.2.0";

  # Link-time NVIDIA libs (libnv_audiofx + libcudart) from the HUSH CDN. Only the
  # daemon links these; the runtime CUDA/TensorRT stack + models are downloaded by
  # the app at first launch, so the store package carries no heavy NVIDIA payload.
  afxLink = fetchurl {
    url = "https://cdn.hush.umceko.com/sdk/2.1.0/afx-link-x86_64.tar.zst";
    hash = "sha256-6zgRNNz654o1uF8GxtLIcQPvu35zbg3LzT2qT8NZ66U=";
  };
  # Unpacked once into the store (symlink chains preserved); build.rs reads it.
  afxLinkDir = runCommandLocal "afx-link" { } ''
    mkdir -p $out
    tar --use-compress-program=${zstd}/bin/zstd -xf ${afxLink} -C $out
  '';

  nativeBuildInputs = [
    pkg-config
    wrapGAppsHook3
    rustPlatform.bindgenHook # pipewire-sys / libspa-sys
    # Also native: libspa-sys/pipewire-sys build.rs run pkg-config for the spa
    # headers, which reads the native PKG_CONFIG_PATH.
    pipewire
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

  commonArgs = {
    inherit pname version buildInputs nativeBuildInputs;
    # libspa-sys bindgen needs the pipewire/spa headers visible at build time;
    # strictDeps hides target buildInputs from the bindgen clang invocation.
    strictDeps = false;
    doCheck = false;

    src = craneLib.cleanCargoSource ../.;

    # Both the deps-only and final builds see the link libs.
    NVAFX_LINK_DIR = "${afxLinkDir}";
    # libspa-sys runs bindgen in build.rs; crane's deps build otherwise produces
    # partial bindings (SPA_ID_INVALID goes missing). Give clang libclang's own
    # headers, the libc headers, and pipewire's spa include dir explicitly.
    LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
    BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${llvmPackages.libclang.lib}/lib/clang/${lib.getVersion llvmPackages.libclang}/include -isystem ${stdenv.cc.libc.dev}/include -I${pipewire.dev}/include/spa-0.2 -I${pipewire.dev}/include/pipewire-0.3";
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (commonArgs // {
  inherit cargoArtifacts;

  # Only the shippable binaries (skip the dev tools denoise/mic/inttest).
  cargoExtraArgs = "-p hush-app -p hush-engine --bin hush --bin hushd";

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
})
