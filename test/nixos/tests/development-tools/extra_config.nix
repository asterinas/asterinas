{ pkgs, ... }:
let
  gdb_sample = pkgs.writeTextFile {
    name = "gdb_sample.c";
    text = builtins.readFile ../../../../book/src/distro/popular-applications/development-tools/gdb_sample.c;
  };
  gdb_commands = pkgs.writeTextFile {
    name = "gdb_commands.gdb";
    text = builtins.readFile ../../../../book/src/distro/popular-applications/development-tools/gdb_commands.gdb;
  };
in
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
    gdb
    strace
    hugo
    direnv
    shellcheck
    jq
    yq-go
  ];

  programs.direnv.enable = true;

  system.activationScripts.testFixtures = ''
    ln -sfT ${gdb_sample} /tmp/gdb_sample.c
    ln -sfT ${gdb_commands} /tmp/gdb_commands.gdb
  '';
}
