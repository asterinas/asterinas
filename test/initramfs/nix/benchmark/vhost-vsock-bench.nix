{ stdenv, glibc, }:
stdenv.mkDerivation {
  pname = "vhost-vsock-bench";
  version = "0.1.0";
  src = ../../src/benchmark/vhost_vsock/vhost_vsock_bench.c;

  dontUnpack = true;
  buildInputs = [ glibc glibc.static ];

  buildPhase = ''
    runHook preBuild

    $CC -std=gnu11 -O2 -Wall -Wextra -Werror -pthread -static \
      "$src" -o vhost_vsock_bench

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin
    mv vhost_vsock_bench $out/bin/

    runHook postInstall
  '';
}
