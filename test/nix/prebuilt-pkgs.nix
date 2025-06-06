{ pkgs ? import <nixpkgs> { }, }:
with pkgs;
let
  inherit busybox;
  inherit fio;
  hackbench = callPackage ./hackbench.nix { };
  iozone = callPackage ./iozone.nix { };
  inherit iperf3;
  inherit libmemcached;
  lmbench = callPackage ./lmbench.nix { };
  ltp = callPackage ./ltp.nix { };
  membench = callPackage ./membench.nix { };
  schbench = callPackage ./schbench.nix { };
  inherit unixbench;
  sysbench =
    if hostPlatform.system == "x86_64-linux" then pkgs.sysbench else "";
in stdenv.mkDerivation {
  pname = "prebuilt-pkgs";
  version = "0.1.0";

  buildCommand = ''
    mkdir -p $out

    ln -s ${busybox} $out/busybox
    ln -s ${fio} $out/fio
    ln -s ${hackbench} $out/hackbench
    ln -s ${iozone} $out/iozone
    ln -s ${iperf3} $out/iperf3
    ln -s ${libmemcached} $out/libmemcached
    ln -s ${lmbench} $out/lmbench
    ln -s ${ltp} $out/ltp
    ln -s ${membench} $out/membench
    ln -s ${schbench} $out/schbench
    ln -s ${unixbench} $out/unixbench

    if [ "${sysbench}" ]; then
      ln -s ${sysbench} $out/sysbench
    fi
  '';
}
