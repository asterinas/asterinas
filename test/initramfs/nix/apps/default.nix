{ lib, pkgs, stdenv, callPackage, testPlatform ? "asterinas", }:
let
  scripts = lib.fileset.toSource {
    root = ./../../src/apps/scripts;
    fileset =
      lib.fileset.fileFilter (file: file.hasExt "sh") ./../../src/apps/scripts;
  };

  commonArgs = { inherit testPlatform; };
  commonBuild = dir: callPackage ./common.nix (commonArgs // { inherit dir; });

  subDirs =
    [ "device" "fs" "hello_world" "io" "ipc" "memory" "process" "security" ];

  tdxAttest = callPackage ./tdx-attest.nix { };

  allPkgs = lib.genAttrs subDirs commonBuild // {
    network = callPackage ./common.nix (commonArgs // {
      dir = "network";
      extraAttrs = { C_FLAGS = "-I${pkgs.libnl.dev}/include/libnl3"; };
      extraBuildInputs = [ pkgs.libnl ];
    });
  } // lib.optionalAttrs (pkgs.hostPlatform.system == "x86_64-linux") {
    intel_tdx = callPackage ./common.nix (commonArgs // {
      dir = "intel_tdx";
      extraAttrs = { TDX_ATTEST_DIR = "${tdxAttest}/QuoteGeneration"; };
    });
  };
in {
  package = stdenv.mkDerivation {
    pname = "apps";
    version = "0.1.0";
    buildCommand = ''
      mkdir -p $out
      cp ${scripts}/* $out

      ${lib.concatMapStringsSep "\n" (name: ''
        ln -sT "${allPkgs.${name}}/${name}" "$out/${name}"
      '') (lib.attrNames allPkgs)}
    '';
  };
}
