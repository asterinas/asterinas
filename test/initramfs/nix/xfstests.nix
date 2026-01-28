{ lib, stdenvNoCC, pkgs }:

let
  whiteListPath = ./../src/fs/xfstests/white.list;
  hasWhiteList = builtins.pathExists whiteListPath;
in
stdenvNoCC.mkDerivation {
  name = "xfstests-package";

  buildCommand = ''
    # Create output directory structure
    mkdir -p $out/xfstests
    mkdir -p $out/bin

    # Copy xfstests test suite from Nix package
    cp -r ${pkgs.xfstests}/lib/xfstests/* $out/xfstests/

    # Copy xfstests binaries
    cp -r ${pkgs.xfstests}/bin/* $out/bin/

    # Create wrapper script for running xfstests
    cat > $out/xfstests/run-xfstests.sh << 'EOF'
#!/bin/sh
export PATH=\
${pkgs.perl}/bin:\
${pkgs.bash}/bin:\
${pkgs.gnugrep}/bin:\
${pkgs.gnused}/bin:\
${pkgs.gawk}/bin:\
${pkgs.coreutils}/bin:\
${pkgs.findutils}/bin:\
${pkgs.bc}/bin:\
${pkgs.kmod}/bin:\
${pkgs.kmod}/sbin:\
${pkgs.xfsprogs}/bin:\
${pkgs.xfsprogs}/sbin:\
${pkgs.e2fsprogs}/bin:\
${pkgs.e2fsprogs}/sbin:\
/bin:\
/usr/bin
cd /xfstests
exec ./check "$@"
EOF
    chmod +x $out/xfstests/run-xfstests.sh

    # Copy local.config from source directory
    cp ${./../src/fs/xfstests/local.config} $out/xfstests/local.config

    # create a mkfs wrapper script
    cat > $out/xfstests/asterinas_mkfs << 'EOF'
#!/bin/sh
REAL_BIN="${pkgs.e2fsprogs}/bin/mke2fs"

DEV=""
for arg in "$@"; do
    case "$arg" in
        /dev/vd*) DEV="$arg" ;;
    esac
done

if [ -n "$DEV" ]; then
    echo "[ASTERINAS-WRAPPER] Forcing 4KB blocks and size for $DEV"
    # 10G image with 4KB blocks = 2621440 blocks
    exec "$REAL_BIN" -t ext2 -F -b 4096 "$DEV" 2621440
else
    exec "$REAL_BIN" -t ext2 "$@"
fi
EOF
    chmod +x $out/xfstests/asterinas_mkfs

    # copy fix.sh into xfstests directory
    cp ${./../src/fs/xfstests/fix.sh} $out/xfstests/fix.sh
    chmod +x $out/xfstests/fix.sh

    # copy skip.list into xfstests directory
    cp ${./../src/fs/xfstests/skip.list} $out/xfstests/skip.list

    # copy white.list into xfstests directory if it exists
    ${lib.optionalString hasWhiteList ''
      cp ${whiteListPath} $out/xfstests/white.list
    ''}
  '';
}
