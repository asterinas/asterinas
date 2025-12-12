{ config, lib, pkgs, ... }:
let
  startXfce =
    pkgs.writeScriptBin "start_xfce" (builtins.readFile ./start_xfce.sh);
in {
  imports = [ ./wallpaper.nix ];

  environment.systemPackages = (lib.optionals (config.services.xserver.enable
    && config.services.xserver.desktopManager.xfce.enable) [ startXfce ])
    ++ (lib.optionals config.services.xserver.enable
      [ pkgs.xorg.xf86videofbdev ]);

  services.displayManager.autoLogin.enable = false;
  services.xserver.displayManager.lightdm.enable = false;

  systemd.services."xfce-desktop" = lib.mkIf (config.services.xserver.enable
    && config.services.xserver.desktopManager.xfce.enable) {
      description = "XFCE Desktop Environment";
      after = [ "getty.target" ];
      wantedBy = [ "multi-user.target" ];
      # XFCE needs exclusive access to tty1 to prevent the getty login prompt
      # from interfering with the graphical display. This conflict ensures
      # that getty@tty1.service does not run alongside the XFCE desktop.
      conflicts = [ "getty@tty1.service" ];
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
