{ lib, stdenvNoCC, pkgs, conformanceSrc }:

let
  standaloneCoreutils = pkgs.coreutils.override { singleBinary = false; };

  runtimeDeps = with pkgs; [
    standaloneCoreutils
    perl
    bash
    gnugrep
    gnused
    gawk
    coreutils
    glibc.bin
    findutils
    util-linux
    bc
    kmod
    xfsprogs
    e2fsprogs
  ];

  sbinDeps = with pkgs; [ util-linux kmod xfsprogs e2fsprogs ];

  runtimePath = lib.makeBinPath runtimeDeps + ":"
    + lib.concatMapStringsSep ":" (package: "${package}/sbin") sbinDeps
    + ":/bin:/usr/bin:/sbin:/usr/sbin";

in stdenvNoCC.mkDerivation {
  name = "xfstests";

  buildCommand = ''
    mkdir -p $out/xfstests
    cp -r ${pkgs.xfstests}/lib/xfstests/* $out/xfstests/
    cp ${conformanceSrc}/xfstests/run_xfstests.sh $out/xfstests/
    sed -i "s|__RUNTIME_PATH__|${runtimePath}|" $out/xfstests/run_xfstests.sh
    chmod +x $out/xfstests/run_xfstests.sh
    cp ${conformanceSrc}/xfstests/local.config $out/xfstests/
    cp ${conformanceSrc}/xfstests/*.list $out/xfstests/
  '';
}
