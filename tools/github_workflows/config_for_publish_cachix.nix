# This configuration file enables all packages and services that are modified
# by Asterinas NixOS. By enabling them, we ensure they are built during the
# evaluation and can subsequently be uploaded to our Cachix binary cache.

{ config, lib, pkgs, ... }:

{
  services.xserver.enable = true;
  services.xserver.desktopManager.xfce.enable = true;

  virtualisation.podman.enable = true;
}
