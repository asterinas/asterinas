final: prev:
let
  libapparmor = prev.libapparmor.overrideAttrs (_: {
    version = "5.0.2";
    src = prev.fetchFromGitLab {
      owner = "apparmor";
      repo = "apparmor";
      tag = "v5.0.2";
      hash = "sha256-Kuc5Nrz4iq5MC5AOcJL9Sb54DWNKlEk3b3DW0vpgSZg=";
    };
  });
in
{
  inherit libapparmor;
  apparmor-parser = (prev.apparmor-parser.override { inherit libapparmor; }).overrideAttrs (old: {
    buildInputs = old.buildInputs ++ [ prev.zstd ];
    postPatch = ''
      patchShebangs .
      cd parser

      substituteInPlace Makefile \
        --replace-fail "/usr/include/linux/capability.h" "${prev.stdenv.cc.libc.linuxHeaders}/include/linux/capability.h"
    '';
  });
}
