{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
stdenv.mkDerivation rec {
  pname = "hackbench";
  version = "0.92";
  src = fetchurl {
    url =
      "https://www.kernel.org/pub/linux/utils/rt-tests/older/rt-tests-${version}.tar.gz";
    hash = "sha256-t310FkJg3yJtxXATFE075oA1hlHb6QAb++uZvW2YMkQ";
  };
  buildPhase = ''
    cd src/hackbench
    make hackbench
  '';
  installPhase = ''
    mkdir -p $out
    cp hackbench $out/
  '';
}
