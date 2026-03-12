{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    zsh
    fish
    htop
    btop
    fastfetch
    coreutils
    util-linux
    procps
    findutils
    less
  ];
}
