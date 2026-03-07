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
  systemd.targets.getty.wants = [
    "autovt@hvc0.service"
    "getty@tty2.service"
    "getty@tty3.service"
    "getty@tty4.service"
    "getty@tty5.service"
    "getty@tty6.service"
  ];

  systemd.extraConfig = ''
    LogLevel=crit      
    ShowStatus=no
  '';
}
