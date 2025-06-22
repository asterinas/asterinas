{ pkgs ? import <nixpkgs> { }, }:
pkgs.libmemcached.overrideAttrs (_: {
  configureFlags = [ "--enable-memaslap" ];
  LDFLAGS = "-lpthread";
  CPPFLAGS = "-fcommon -fpermissive";
})
