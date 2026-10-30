{ lib, pkgs, ... }:

{
  environment.systemPackages = [ pkgs.apparmor-parser ];

  system.systemBuilderCommands = lib.mkAfter ''
    sed -i 's_ -- root=_ lsm=apparmor,yama -- root=_' $out/kernel-params
  '';
}
