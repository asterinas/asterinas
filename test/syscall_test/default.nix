{ lib
, stdenv
, fetchFromGitHub
, callPackage
, makeWrapper
}:
let
  syscall-tests = callPackage ./all.nix { };
in stdenv.mkDerivation rec {
  name = "syscall-tests-for-initrd";

  src = ./.;

  buildInputs = [ syscall-tests ];

  ASTER_PREBUILT_SYSCALL_TEST = "${syscall-tests}/bin";

  buildPhase = "true";
  installPhase = ''
    make TARGET_DIR=$out
  '';
  dontPatchShebangs = true;
}
