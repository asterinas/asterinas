{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    fish
    zsh
    busybox
    fastfetch
    lsof
    ncdu
    procps
    coreutils
    diffutils
    findutils
    gnugrep
    net-tools
    less
    man-pages
    util-linux
    which
  ];
}
