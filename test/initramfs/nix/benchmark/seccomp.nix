# SPDX-License-Identifier: MPL-2.0

{
  stdenv,
  glibc,
}:
stdenv.mkDerivation {
  pname = "seccomp-bench";
  version = "0.1.0";
  src = ./../../src/benchmark/seccomp/syscall_latency/bench.c;
  buildInputs = [ glibc.static ];
  buildCommand = ''
    mkdir -p $out/bin
    $CC -O2 -Wall -Werror -static $src -o $out/bin/seccomp-bench
  '';
}
