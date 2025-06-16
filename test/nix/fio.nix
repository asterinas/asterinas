{ pkgs ? import <nixpkgs> { }, }:
pkgs.fio.overrideAttrs (_: { configureFlags = [ "--esx" ]; })
