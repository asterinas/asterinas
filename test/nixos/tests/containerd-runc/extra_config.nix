{ config, lib, pkgs, ... }:

{
  hardware.enableRedistributableFirmware = lib.mkForce false;

  virtualisation.containerd.enable = true;

  environment.systemPackages = with pkgs; [
    runc
  ];
}
