{ pkgs, ... }:
let
  drm_xorg_mesa_smoke = pkgs.writeShellScript "drm_xorg_mesa_smoke.sh" (
    builtins.readFile ./drm_xorg_mesa_smoke.sh
  );
in
{
  hardware.graphics.enable = true;
  services.xserver.enable = true;
  services.xserver.desktopManager.xfce.enable = true;

  environment.systemPackages = with pkgs; [
    coreutils
    mesa-demos
    xrandr
  ];

  system.activationScripts.testFixtures = ''
    ln -sfT ${drm_xorg_mesa_smoke} /tmp/drm_xorg_mesa_smoke.sh
  '';
}
