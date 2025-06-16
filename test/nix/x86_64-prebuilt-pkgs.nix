{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
let
  inherit busybox;
  fio = callPackage ./fio.nix { };
  hackbench = callPackage ./hackbench.nix { };
  inherit iperf3;
  lmbench = callPackage ./lmbench.nix { };
  ltp = callPackage ./ltp.nix { };
  inherit memcached;
  inherit nginx;
  inherit redis;
  schbench = callPackage ./schbench.nix { };
  sqlite-speedtest1 = callPackage ./sqlite-speedtest1.nix { };
  inherit sysbench;
in stdenv.mkDerivation {
  pname = "x86_64-prebuilt-pkgs";
  version = "0.1.0";

  buildCommand = ''
    mkdir -p $out
    ln -s ${busybox} $out/busybox
    ln -s ${fio} $out/fio
    ln -s ${hackbench} $out/hackbench
    ln -s ${iperf3} $out/iperf3
    ln -s ${lmbench} $out/lmbench
    ln -s ${ltp} $out/ltp
    ln -s ${memcached} $out/memcached
    ln -s ${nginx} $out/nginx
    ln -s ${redis} $out/redis
    ln -s ${schbench} $out/schbench
    ln -s ${sqlite-speedtest1} $out/sqlite-speedtest1
    ln -s ${sysbench} $out/sysbench
  '';
}
