{ pkgs ? import <nixpkgs> { }, extra-substituters ? ""
, extra-trusted-public-keys ? "", ... }:
let
  installer = pkgs.callPackage ../aster_nixos_installer {
    inherit extra-substituters extra-trusted-public-keys;
  };
  nixos = pkgs.nixos (import "${installer}/etc_nixos/configuration.nix");
  cachixPkgs = with nixos.pkgs;
    [
      hello-asterinas
      xfce.xfdesktop
      xfce.xfwm4
      xorg.xorgserver
      runc
      runc.man
      podman
      podman.man
      aster_systemd
    ] ++ (with nixos.config; [
      system.build.toplevel
      systemd.package
      systemd.package.debug
      systemd.package.dev
      systemd.package.man
      virtualisation.podman.package
      virtualisation.podman.package.man
    ]);
in pkgs.writeClosure cachixPkgs
