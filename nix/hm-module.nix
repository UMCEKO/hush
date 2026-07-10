# Home-manager module: adds the package and runs hushd as a user service.
#   imports = [ hush.homeManagerModules.default ];
#   services.hush.enable = true;
flake:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.hush;
in
{
  options.services.hush = {
    enable = lib.mkEnableOption "HUSH denoiser daemon (NVIDIA Maxine virtual microphone)";
    package = lib.mkOption {
      type = lib.types.package;
      default = flake.packages.${pkgs.system}.hush;
      description = "The hush package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    systemd.user.services.hush = {
      Unit = {
        Description = "HUSH denoiser daemon (NVIDIA Maxine virtual microphone)";
        # PipeWire is a user-session service, so hushd must be one too.
        After = [ "pipewire.service" "wireplumber.service" ];
        Wants = [ "pipewire.service" ];
      };
      Service = {
        ExecStart = "${cfg.package}/bin/hushd";
        Restart = "on-failure";
        RestartSec = 2;
      };
      Install.WantedBy = [ "default.target" ];
    };
  };
}
