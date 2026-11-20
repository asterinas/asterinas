# SPDX-License-Identifier: MPL-2.0

{ lib, pkgs, ... }:

let
  busyboxRoot = pkgs.runCommand "kata-tcg-busybox-root" { } ''
    mkdir -p "$out/bin"
    cp ${pkgs.pkgsStatic.busybox}/bin/busybox "$out/bin/busybox"
    chmod 0755 "$out/bin/busybox"
    ln -s busybox "$out/bin/sh"
  '';
  busyboxImage = pkgs.dockerTools.buildImage {
    name = "busybox";
    tag = "latest";
    copyToRoot = busyboxRoot;
    config.Cmd = [ "/bin/sh" ];
  };
  # Normalize dockerTools' gzip stream for ctr's local archive importer.
  busyboxTar = pkgs.runCommand "kata-tcg-busybox.tar" { } ''
    ${pkgs.gzip}/bin/gzip -dc ${busyboxImage} > "$out"
  '';
in {
  aster_nixos.kata-tcg.enable = true;
  hardware.enableRedistributableFirmware = lib.mkForce false;
  environment.etc."kata-tcg/busybox.tar".source = busyboxTar;
  systemd.services.containerd.serviceConfig = {
    StandardOutput = "append:/var/log/containerd.log";
    StandardError = "append:/var/log/containerd.log";
  };
}
