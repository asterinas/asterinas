{ config, lib, pkgs, ... }:
let
  runAsXfce = pkgs.writeScriptBin "run_as_xfce" (builtins.readFile ./run_as_xfce.sh);
in
{
  environment.systemPackages =
    (lib.optionals (config.services.xserver.enable &&
      config.services.xserver.desktopManager.xfce.enable)
      [ runAsXfce ])
    ++ (lib.optionals config.services.xserver.enable
      [ pkgs.xorg.xf86videofbdev ])
    ++ [ pkgs.vim ];

  systemd.services."xfce-desktop" = lib.mkIf
    (config.services.xserver.enable && config.services.xserver.desktopManager.xfce.enable)
    {
      description = "XFCE Desktop Environment";
      after = [ "multi-user.target" ];
      wantedBy = [ "graphical.target" ];
      serviceConfig = {
        Environment = "DISPLAY=:0";
        ExecStart = "${runAsXfce}/bin/run_as_xfce";
        StandardOutput = "tty";
        StandardError = "tty";
        KillMode = "process";
        Delegate = "yes";
        Restart = "no";
        Type = "simple";
      };
    };
}