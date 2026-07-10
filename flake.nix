{
  description = "HUSH — NVIDIA Maxine denoiser virtual microphone for PipeWire";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Local forks (committed state only — commit before `nix flake update`).
    # dioxus: git main relabelled for the Blitz alpha.5 stack; blitz: main + local renderer fixes.
    dioxus-src = {
      url = "git+file:///home/umceko/dioxus";
      flake = false;
    };
    blitz-src = {
      url = "git+file:///home/umceko/blitz";
      flake = false;
    };

    # Proprietary NVIDIA Maxine Audio Effects SDK (user-downloaded, NOT redistributable —
    # it only enters the local store, never a public cache).
    nvafx-sdk = {
      url = "path:/home/umceko/maxine-dl/sdk/Audio_Effects_SDK";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, dioxus-src, blitz-src, nvafx-sdk }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true; # Maxine AFX SDK
      };
      hush = pkgs.callPackage ./nix/package.nix {
        inherit dioxus-src blitz-src nvafx-sdk;
      };
    in
    {
      packages.${system} = {
        default = hush;
        inherit hush;
      };

      # Optional: `programs.hush.enable` style module for home-manager.
      homeManagerModules.default = import ./nix/hm-module.nix self;

      devShells.${system}.default = pkgs.mkShell {
        inputsFrom = [ hush ];
        env.NVAFX_SDK = "${nvafx-sdk}";
      };
    };
}
