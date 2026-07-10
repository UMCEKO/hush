{
  description = "HUSH — NVIDIA Maxine denoiser virtual microphone for PipeWire";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true; # the CDN link tarball carries NVIDIA libs
      };
      hush = pkgs.callPackage ./nix/package.nix { };

      # Native libs the webview GUI + PipeWire engine need to build/link.
      buildDeps = with pkgs; [
        pipewire dbus webkitgtk_4_1 gtk3 libsoup_3 glib glib-networking openssl xdotool zstd
      ];
    in
    {
      packages.${system} = {
        default = hush;
        inherit hush;
      };

      homeManagerModules.default = import ./nix/hm-module.nix self;

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook3 rustPlatform.bindgenHook rustc cargo ];
        buildInputs = buildDeps;
        # Dev builds link the engine against the locally-extracted SDK; packaged
        # builds use NVAFX_LINK_DIR from the CDN link tarball instead.
        NVAFX_SDK = "/home/umceko/maxine-dl/sdk/Audio_Effects_SDK";
      };
    };
}
