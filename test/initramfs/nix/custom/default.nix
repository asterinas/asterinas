{ lib, stdenv, callPackage, testPlatform ? "asterinas", intelTdx ? 0, }: rec {
  scripts = lib.fileset.toSource {
    root = ./../../src/custom/scripts;
    fileset = lib.fileset.fileFilter (file: file.hasExt "sh")
      ./../../src/custom/scripts;
  };

  buildDir = dir:
    callPackage ./common.nix { inherit dir testPlatform intelTdx; };
  subDirs = [
    "device"
    "examples"
    "fs"
    "io"
    "ipc"
    "memory"
    "network"
    "process"
    "security"
  ];
  subPkgs = map buildDir subDirs;

  package = stdenv.mkDerivation {
    pname = "custom-tests";
    version = "0.1.0";
    buildCommand = ''
      mkdir -p $out
      cp ${scripts}/* $out
      ${lib.concatMapStringsSep "\n" (index:
        let
          dir = builtins.elemAt subDirs index;
          pkg = builtins.elemAt subPkgs index;
        in "ln -s ${pkg}/${dir} $out/${dir}")
      (lib.range 0 (builtins.length subDirs - 1))}
    '';
  };
}
