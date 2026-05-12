{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    bzip2
    gzip
    p7zip
    gnutar
    xz
    zip
    unzip
    screen
    file
    bat
    gawk
    sd
    gnused
    eza
    fd
    fzf
    ripgrep
    silver-searcher
    tree
    age
    crunch
    gnupg
    john
    restic
    wipe
  ];
}
