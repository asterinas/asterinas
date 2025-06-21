{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
stdenv.mkDerivation {
  pname = "apps";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./..;
    fileset = ../apps;
  };

  MONGOOSE_DIR = fetchFromGitHub {
    owner = "cesanta";
    repo = "mongoose";
    rev = "7.13";
    sha256 = "sha256-9XHUE8SVOG/X7SIB52C8EImPx4XZ7B/5Ojwmb0PkiuI";
  };

  HOST_PLATFORM = "${hostPlatform.system}";
  CC = "${stdenv.cc.targetPrefix}cc";
  C_FLAGS = "-I${libnl.dev}/include/libnl3";
  buildInputs = [ glibc glibc.static libnl ];
  buildCommand = ''
    BUILD_DIR=$(mktemp -d)
    mkdir -p $BUILD_DIR/build
    cp -r $src/apps $BUILD_DIR/

    pushd $BUILD_DIR
    make --no-print-directory -C apps
    popd

    mkdir -p $out/test
    mv $BUILD_DIR/build/initramfs/test $out/
  '';
}
