{ stdenv, fetchurl, }:
stdenv.mkDerivation rec {
  pname = "hackbench";
  version = "0.92";
  src = fetchurl {
    url =
      "https://www.kernel.org/pub/linux/utils/rt-tests/older/rt-tests-${version}.tar.gz";
    hash = "sha256-t310FkJg3yJtxXATFE075oA1hlHb6QAb++uZvW2YMkQ";
  };

  buildPhase = ''
    runHook preBuild

    cd src/hackbench
    make hackbench

    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin
    mv hackbench $out/bin/

    runHook postInstall
  '';
}
