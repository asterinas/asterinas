{ lib, stdenvNoCC, pkgs, conformanceSrc }:

let
  xfstests = pkgs.xfstests.overrideAttrs (old: rec {
    version = "2026.06.21";
    src = pkgs.fetchzip {
      url =
        "https://git.kernel.org/pub/scm/fs/xfs/xfstests-dev.git/snapshot/xfstests-dev-v${version}.tar.gz";
      hash = "sha256-hngS9Hnsz9XKQ42yh6mcXHiTOzL+Zk9hRpai7e2tU0E=";
    };
    nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ pkgs.pkg-config ];
  });

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
    cp -r ${xfstests}/lib/xfstests/* $out/xfstests/
    cp ${conformanceSrc}/xfstests/run_xfstests.sh $out/xfstests/
    sed -i "s|__RUNTIME_PATH__|${runtimePath}|" $out/xfstests/run_xfstests.sh
    chmod +x $out/xfstests/run_xfstests.sh
    cp ${conformanceSrc}/xfstests/local.config $out/xfstests/
    cp ${conformanceSrc}/xfstests/*.list $out/xfstests/
  '';
}
