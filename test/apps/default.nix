{ lib
, stdenv
, fetchFromGitHub
, callPackage
, makeWrapper
, glibc
}:
stdenv.mkDerivation rec {
  name = "initrd-apps";

  src = ./.;
  MONGOOSE_DIR = fetchFromGitHub {
    owner = "cesanta";
    repo = "mongoose";
    rev = "7.13";
    sha256 = "sha256-9XHUE8SVOG/X7SIB52C8EImPx4XZ7B/5Ojwmb0PkiuI";
  };

  buildInputs = [ glibc glibc.static ];

  enableParallelBuilding = true;

  buildPhase = ''
    make BUILD_DIR=$(pwd)/build INSTALL_DIR=$out
  '';
  installPhase = "true";
  dontPatchShebangs = true;
}
