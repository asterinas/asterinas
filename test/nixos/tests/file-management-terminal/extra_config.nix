{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    gnutar
    gzip
    bzip2
    xz
    p7zip
    zip
    unzip
    screen
    file
    gawk
    gnused
    fzf
    ripgrep
    fd
    bat
    eza
  ];
}
