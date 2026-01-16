{ lib, pkgs, stdenv, dir, testPlatform ? "asterinas", intelTdx ? 0, }:
let
  mongoose =
    if dir == "examples" then pkgs.callPackage ./mongoose.nix { } else null;
  tdxAttest =
    if intelTdx != 0 then pkgs.callPackage ./tdx-attest.nix { } else null;
in stdenv.mkDerivation {
  pname = "${dir}-test";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./../../src/custom;
    fileset = lib.fileset.unions [
      ./../../src/custom/common
      ./../../src/custom/${dir}
    ];
  };

  HOST_PLATFORM = "${pkgs.hostPlatform.system}";
  TEST_PLATFORM = "${testPlatform}";

  MONGOOSE_DIR = lib.optionalString (dir == "examples") "${mongoose}";

  INTEL_TDX = "${toString intelTdx}";
  TDX_ATTEST_DIR =
    lib.optionalString (intelTdx != 0) "${tdxAttest}/QuoteGeneration";

  CC = "${stdenv.cc.targetPrefix}cc";
  C_FLAGS =
    lib.optionalString (dir == "network") " -I${pkgs.libnl.dev}/include/libnl3";

  buildInputs = with pkgs;
    [ glibc glibc.static ] ++ lib.optionals (dir == "network") [ libnl ];
  buildCommand = ''
    mkdir -p $out
    make --no-print-directory BUILD_DIR=$(mktemp -d) OUTPUT_DIR=$out -C "$src/${dir}"
  '';
}
