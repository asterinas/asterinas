{ config, lib, pkgs, ... }:

{
  systemd.package = pkgs.aster_systemd;

  # TODO: The following services currently do not work and 
  # may affect systemd startup or cause performance issues. 
  # Enable them after they can run successfully.
  systemd.coredump.enable = false;
  systemd.oomd.enable = false;
  systemd.services.logrotate.enable = false;
  systemd.services.network-setup.enable = false;
  systemd.services.resolvconf.enable = false;
  systemd.services.systemd-random-seed.enable = false;
  systemd.services.systemd-tmpfiles-clean.enable = false;
  systemd.services.systemd-tmpfiles-setup.enable = false;
  services.timesyncd.enable = false;
  services.udev.enable = false;

  services.getty.autologinUser = "root";
  users.users.root = {
    shell = "${pkgs.bash}/bin/bash";
    hashedPassword = null;
  };

  systemd.targets.getty.wants =
    # tty1: provide text login ONLY when X server is disabled.
    # Other VTs: always provide text logins
    (lib.optional (!config.services.xserver.enable) "autovt@tty1.service") ++ [
      "autovt@hvc0.service"
      "autovt@tty2.service"
      "autovt@tty3.service"
      "autovt@tty4.service"
      "autovt@tty5.service"
      "autovt@tty6.service"
    ];

  systemd.extraConfig = ''
    LogLevel=crit      
    ShowStatus=no
  '';
}
