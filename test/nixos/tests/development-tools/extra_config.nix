{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    (python3.withPackages (p: [ p.meson ]))
    nodejs
    go
    cargo
    rustc
    openjdk
    ruby
    perl
    lua
    php
    gcc
    llvmPackages.clangUseLLVM
    git
    gnumake
    cmake
    meson
    ninja
    vim
    neovim
    emacs
    nano
  ];
}
