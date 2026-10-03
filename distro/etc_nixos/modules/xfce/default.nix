{
  config,
  lib,
  pkgs,
  ...
}:
let
  startXfce = pkgs.writeScriptBin "start_xfce" (
    builtins.replaceStrings
      [ "@runtime_shell@" "@xfce_xinitrc@" ]
      [ pkgs.runtimeShell pkgs.xfce4-session.xinitrc ]
      (builtins.readFile ./start_xfce.sh)
  );
in
{
  imports = [ ./wallpaper.nix ];

  environment.systemPackages =
    (lib.optionals (
      config.services.xserver.enable && config.services.xserver.desktopManager.xfce.enable
    ) [ startXfce ])
    ++ (lib.optionals config.services.xserver.enable [
      pkgs.xf86-video-fbdev
      pkgs.xkeyboard-config
    ]);

  services.displayManager.autoLogin.enable = false;
  services.xserver.displayManager.lightdm.enable = false;

  systemd.services."xfce-desktop" =
    lib.mkIf (config.services.xserver.enable && config.services.xserver.desktopManager.xfce.enable)
      {
        description = "XFCE Desktop Environment";
        wantedBy = [ "multi-user.target" ];
        # XFCE needs exclusive access to tty1 to prevent the getty login prompt
        # from interfering with the graphical display. This conflict ensures
        # that getty@tty1.service does not run alongside the XFCE desktop.
        conflicts = [ "getty@tty1.service" ];
        serviceConfig = {
          # This desktop runs as root without a login session. Applications need
          # HOME to locate writable user data instead of the read-only Nix store.
          Environment = [
            "DISPLAY=:0"
            "HOME=/root"
          ];
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
