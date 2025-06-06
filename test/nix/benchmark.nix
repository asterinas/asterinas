{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
let
  inherit fio;
  hackbench = callPackage ./hackbench.nix { };
  iozone = callPackage ./iozone.nix { };
  inherit iperf3;
  inherit libmemcached;
  lmbench = callPackage ./lmbench.nix { };
  membench = callPackage ./membench.nix { };
  schbench = callPackage ./schbench.nix { };
  inherit unixbench;
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
    cp -r ${hackbench}/* $out/benchmark/bin/
    cp -r ${iozone}/* $out/benchmark/bin/
    cp -r ${iperf3}/bin/iperf3 $out/benchmark/bin/
    cp -r ${libmemcached}/bin/* $out/benchmark/bin/
    cp -r ${lmbench}/* $out/benchmark/bin/
    cp -r ${membench}/* $out/benchmark/bin/
    cp -r ${schbench}/* $out/benchmark/bin/
    cp -r ${unixbench}/bin/* $out/benchmark/bin/

    if [ "${sysbench}" ]; then
      cp -r ${sysbench}/bin/* $out/benchmark/bin/
    fi

    cp -r $src/benchmark/* $out/benchmark/
  '';
}
