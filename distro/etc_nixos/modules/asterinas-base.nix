# Asterinas NixOS base module.
#
# Provides common settings shared by both disk and ISO.  The universal
# stage-1 init is defined in asterinas-stage-1-init.nix and referenced
# by both core.nix (disk) and iso_image/default.nix (ISO).
{ config, lib, pkgs, ... }:
let
  stage-1-init = import ./asterinas-stage-1-init.nix { inherit pkgs; };

  initramfs = pkgs.makeInitrd {
    contents = [
      {
        object = "${pkgs.busybox}/bin";
        symlink = "/bin";
      }
      {
        object = stage-1-init;
        symlink = "/init";
      }
    ];
  };
in {
  # Export the stage-1 init and initramfs as packages so that
  # core.nix (disk) and iso_image (ISO) can both reference them.
  nixpkgs.overlays = [
    (final: prev: {
      asterinas-stage-1-init = stage-1-init;
      asterinas-initramfs = initramfs;
    })
  ];

  # Common settings shared by disk and ISO.
  boot.kernel.enable = false;
  boot.initrd.enable = false;
  system.activationScripts.modprobe = lib.mkForce "";

  # Suppress error and warning messages of systemd.
  # TODO: Fix errors and warnings from systemd and remove this setting.
  environment.sessionVariables = { SYSTEMD_LOG_LEVEL = "crit"; };

  # FIXME: Currently, during `nixos-rebuild`, `texinfo/install-info` encounters a `SIGBUS`.
  documentation.info.enable = false;
}
