{ lib, stdenv, fetchFromGitHub, hostPlatform, glibc, libnl, callPackage
, testPlatform, }: rec {

  tdxAttest = lib.optionalAttrs (builtins.getEnv "INTEL_TDX" == "1")
    (callPackage ./tdx-attest.nix { });

  mongoose_src = fetchFromGitHub {
    owner = "cesanta";
    repo = "mongoose";
    rev = "7.13";
    sha256 = "sha256-9XHUE8SVOG/X7SIB52C8EImPx4XZ7B/5Ojwmb0PkiuI";
  };

  package = stdenv.mkDerivation {
    pname = "apps";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./../src;
      fileset = ./../src/apps;
    };

    MONGOOSE_DIR = "${mongoose_src}";

    INTEL_TDX = builtins.getEnv "INTEL_TDX";
    TDX_ATTEST_DIR = lib.optionalString (builtins.getEnv "INTEL_TDX" == "1")
      "${tdxAttest}/QuoteGeneration";

    HOST_PLATFORM = "${hostPlatform.system}";
    CC = "${stdenv.cc.targetPrefix}cc";
    C_FLAGS = "-I${libnl.dev}/include/libnl3"
      + (if testPlatform == "asterinas" then " -D__asterinas__" else "");
    # FIXME: Excluding `glibc` allows the build to succeed, but causes some tests to fail.
    buildInputs = [ glibc glibc.static libnl ];
    buildCommand = ''
      BUILD_DIR=$(mktemp -d)
      mkdir -p $BUILD_DIR
      cp -r $src/apps $BUILD_DIR/

      pushd $BUILD_DIR
      make --no-print-directory -C apps
      popd

      mkdir -p $out
      mv build/initramfs/test/* $out/
    '';
  };
}
