{ config, lib, pkgs, ... }:

{
  systemd.package = pkgs.aster_systemd;

  # TODO: The following services currently do not work and
  # may affect systemd startup or cause performance issues.
  # Enable them after they can run successfully.
  networking.resolvconf.enable = false;
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

  systemd.targets.getty.wants = lib.mkForce (
    # tty1: provide text login on the virtual console when X server is disabled.
    (lib.optional
      (!config.services.xserver.enable && config.aster_nixos.console == "tty0")
      "autovt@tty1.service")
    ++ lib.optional (config.aster_nixos.console == "hvc0")
    "getty@hvc0.service");

  systemd.settings.Manager = {
    LogLevel = "crit";
    ShowStatus = "no";
  };
}
