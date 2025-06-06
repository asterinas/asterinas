{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
stdenv.mkDerivation rec {
  pname = "iozone";
  version = "3.507";
  src = fetchurl {
    url = "http://www.iozone.org/src/current/iozone${
        lib.replaceStrings [ "." ] [ "_" ] version
      }.tar";
    hash = "sha256-HoCHraBW9dgBjuC8dmhtQW/CJR7QMDgFXb0K940eXOM";
  };

  preBuild = "pushd src/current";
  postBuild = "popd";

  buildFlags = if stdenv.hostPlatform.system == "x86_64-linux" then
    "linux-AMD64"
  else if stdenv.hostPlatform.system == "riscv64-linux" then
    "linux-AMD64"
  else
    throw "Platform ${stdenv.hostPlatform.system} not yet supported.";

  makeFlags = [ "CC=${stdenv.cc.targetPrefix}cc" ];

  # The makefile doesn't define a rule for e.g. libbif.o
  # Make will try to evaluate implicit built-in rules for these outputs if building in parallel
  # Build in serial so that the main rule builds everything before the implicit ones are attempted
  enableParallelBuilding = false;

  installPhase = ''
    mkdir -p $out
    cp src/current/iozone $out/
  '';
}
