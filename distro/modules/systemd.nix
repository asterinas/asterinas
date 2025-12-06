{ config, lib, pkgs, ... }:

{
  systemd.package = pkgs.aster_systemd;

  systemd.coredump.enable = false;
  systemd.services.logrotate.enable = false;
  systemd.services.systemd-tmpfiles-clean.enable = false;
  systemd.services.systemd-tmpfiles-setup.enable = false;
  systemd.services.systemd-random-seed.enable = false;
  systemd.oomd.enable = false;
  services.timesyncd.enable = false;
  services.udev.enable = false;

  services.getty.autologinUser = "root";
  users.users.root = {
    shell = "${pkgs.bash}/bin/bash";
    hashedPassword = null;
  };
  systemd.targets.getty.wants = [ "autovt@hvc0.service" ];

  systemd.extraConfig = ''
    LogLevel=crit      
    ShowStatus=no
  '';
}
