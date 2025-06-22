{ pkgs ? import <nixpkgs> { }, }:
(pkgs.redis.overrideAttrs (_: { doCheck = false; })).override {
  withSystemd = false;
}
