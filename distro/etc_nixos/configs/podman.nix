# configs/podman.nix
{ config, lib, pkgs, ... }:

{
  virtualisation.podman.enable = true;

  environment.systemPackages = with pkgs; [ test-asterinas ];
}
