{ stdenvNoCC, pkgs, kmod ? pkgs.kmod }:

let xfstestSrc = ./../../src/fs/xfstests;
in stdenvNoCC.mkDerivation {
  name = "xfstests-package";

  buildCommand = ''
        # Create output directory structure
        mkdir -p $out/xfstests
        mkdir -p $out/bin

        # Prefer kmod's modprobe over busybox for xfstests.
        cp -r ${kmod}/bin/* $out/bin/
        if [ -d ${kmod}/sbin ]; then
          mkdir -p $out/sbin
          cp -r ${kmod}/sbin/* $out/sbin/
        fi

        # Create wrapper script for running xfstests
        cat > $out/xfstests/run_xfstests.sh << 'EOF'
    #!/bin/sh
    export PATH=\
    ${pkgs.perl}/bin:\
    ${pkgs.bash}/bin:\
    ${pkgs.gnugrep}/bin:\
    ${pkgs.gnused}/bin:\
    ${pkgs.gawk}/bin:\
    ${pkgs.coreutils}/bin:\
    ${pkgs.findutils}/bin:\
    ${pkgs.util-linux}/bin:\
    ${pkgs.util-linux}/sbin:\
    ${pkgs.bc}/bin:\
    ${kmod}/bin:\
    ${kmod}/sbin:\
    ${pkgs.xfsprogs}/bin:\
    ${pkgs.xfsprogs}/sbin:\
    ${pkgs.e2fsprogs}/bin:\
    ${pkgs.e2fsprogs}/sbin:\
    /bin:\
    /usr/bin:\
    /sbin:\
    /usr/sbin
    cd /xfstests
    RUNLIST_FILE=""
    ARGS=""
    while [ $# -gt 0 ]; do
      case "$1" in
        -R|--runlist)
          RUNLIST_FILE="$2"
          shift 2
          ;;
        --)
          shift
          break
          ;;
        *)
          ARGS="$ARGS \"$1\""
          shift
          ;;
      esac
    done
    for arg in "$@"; do
      ARGS="$ARGS \"$arg\""
    done
    if [ -n "$RUNLIST_FILE" ]; then
      if [ ! -f "$RUNLIST_FILE" ]; then
        echo "Run list file not found: $RUNLIST_FILE" >&2
        exit 2
      fi
      while IFS= read -r test; do
        case "$test" in
          ""|\#*) continue ;;
        esac
        ARGS="$ARGS \"$test\""
      done < "$RUNLIST_FILE"
    fi

    ./fix.sh
    # shellcheck disable=SC2086
    eval ./check $ARGS
    echo "All xfstests passed."
    EOF
        chmod +x $out/xfstests/run_xfstests.sh

        # copy fix.sh into xfstests directory
        cp ${xfstestSrc}/fix.sh $out/xfstests/fix.sh
        chmod +x $out/xfstests/fix.sh

        # Copy local.config from source directory
        cp ${xfstestSrc}/local.config $out/xfstests/local.config

        # copy block.list into xfstests directory
        cp ${xfstestSrc}/block.list $out/xfstests/block.list

        # copy run.list into xfstests directory
        cp ${xfstestSrc}/run.list $out/xfstests/run.list

  '';
}
