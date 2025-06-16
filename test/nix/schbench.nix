{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
stdenv.mkDerivation rec {
  pname = "schbench";
  version = "v1.0";
  src = fetchgit {
    url = "https://git.kernel.org/pub/scm/linux/kernel/git/mason/schbench.git";
    rev = "${version}";
    hash = "sha256-BSGp2TpNh29OsqwDEwaRC1W8T6QFec7AhgVgNEslHZY";
  };

  patchPhase = ''
    substituteInPlace schbench.c \
      --replace "defined(__powerpc64__)" "defined(__powerpc64__) || defined(__riscv)"
  '';
  makeFlags = [ "CC=${stdenv.cc.targetPrefix}cc" ];
  installPhase = ''
    mkdir -p $out/bin
    mv schbench $out/bin/
  '';
}
