# configs/nix-commands.nix
{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ test-asterinas ];
}
