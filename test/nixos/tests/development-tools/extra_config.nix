{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    clang
    gcc
    go
    lua
    nodejs
    octave
    openjdk
    perl
    php
    (python3.withPackages (p: [ p.meson ]))
    ruby
    rustc
    git
    cargo
    cmake
    gnumake
    meson
    ninja
    hugo
    direnv
    shellcheck
    jq
    yq-go
  ];

  programs.direnv.enable = true;
}
