{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    fish
    zsh
    fastfetch
    lsof
    ncdu
    procps
    coreutils
    diffutils
    findutils
    gnugrep
    hostname
    less
    man-pages
    util-linux
    which
  ];
}
