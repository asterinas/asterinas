{ pkgs, ... }:
{
  hardware.graphics.enable = true;
  services.xserver.enable = true;
  services.xserver.desktopManager.xfce.enable = true;

  environment.systemPackages = with pkgs; [
    coreutils
    mesa-demos
    xrandr
  ];

}
