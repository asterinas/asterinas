{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
let
  fio = callPackage ./fio.nix { };
  hackbench = callPackage ./hackbench.nix { };
  inherit iperf3;
  lmbench = callPackage ./lmbench.nix { };
  inherit memcached;
  inherit nginx;
  inherit redis;
  schbench = callPackage ./schbench.nix { };
  sqlite-speedtest1 = callPackage ./sqlite-speedtest1.nix { };
  sysbench =
    if hostPlatform.system == "x86_64-linux" then pkgs.sysbench else "";
in stdenv.mkDerivation {
  pname = "benchmark";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./..;
    fileset = ../benchmark;
  };

  buildCommand = ''
    mkdir -p $out/benchmark/bin
    cp -r ${fio}/bin/fio $out/benchmark/bin/
    cp -r ${hackbench}/bin/hackbench $out/benchmark/bin/
    cp -r ${iperf3}/bin/iperf3 $out/benchmark/bin/
    cp -r ${memcached}/bin/memcached $out/benchmark/bin/
    cp -r ${nginx}/bin/nginx $out/benchmark/bin/
    cp -r ${redis}/bin/redis-server $out/benchmark/bin/
    cp -r ${schbench}/bin/schbench $out/benchmark/bin/
    cp -r ${sqlite-speedtest1}/bin/sqlite-speedtest1 $out/benchmark/bin/

    mkdir -p $out/benchmark/bin/lmbench
    cp -r ${lmbench}/bin/* $out/benchmark/bin/lmbench/

    mkdir -p $out/benchmark/nginx/conf
    cp -r ${nginx}/conf/* $out/benchmark/nginx/conf/

    if [ "${sysbench}" ]; then
      cp -r ${sysbench}/bin/sysbench $out/benchmark/bin/
    fi

    cp -r $src/benchmark/* $out/benchmark/
  '';
}
