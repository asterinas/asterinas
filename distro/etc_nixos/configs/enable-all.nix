# configs/enable-all.nix
{ config, lib, pkgs, ... }:

{
  services.xserver.enable = true;
  services.xserver.desktopManager.xfce.enable = true;

  virtualisation.podman.enable = true;
}
