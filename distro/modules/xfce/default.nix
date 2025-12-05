{ config, lib, pkgs, ... }:
let
  startXfce =
    pkgs.writeScriptBin "start_xfce" (builtins.readFile ./start_xfce.sh);
in {
  environment.systemPackages = (lib.optionals (config.services.xserver.enable
    && config.services.xserver.desktopManager.xfce.enable) [ startXfce ])
    ++ (lib.optionals config.services.xserver.enable
      [ pkgs.xorg.xf86videofbdev ]);

  systemd.services."xfce-desktop" = lib.mkIf (config.services.xserver.enable
    && config.services.xserver.desktopManager.xfce.enable) {
      description = "XFCE Desktop Environment";
      after = [ "multi-user.target" ];
      wantedBy = [ "graphical.target" ];
      serviceConfig = {
        Environment = "DISPLAY=:0";
        ExecStart = "${startXfce}/bin/start_xfce";
        StandardOutput = "tty";
        StandardError = "tty";
        KillMode = "process";
        Delegate = "yes";
        Restart = "no";
        Type = "simple";
      };
    };
}
