{ lib, stdenvNoCC, callPackage, hostPlatform, pkgsHostTarget, pkgsBuildBuild
, benchmarkName ? "none", }: rec {
  # Use `--esx` flag to enable `CONFIG_NO_SHM` and disable `CONFIG_HAVE_TIMERFD_CREATE`.
  fio = pkgsHostTarget.fio.overrideAttrs (_: { configureFlags = [ "--esx" ]; });
  hackbench = callPackage ./hackbench.nix { };
  iperf3 = pkgsHostTarget.iperf3;
  lmbench = callPackage ./lmbench.nix { };
  memcached = pkgsHostTarget.memcached;
  nginx = pkgsHostTarget.nginx;
  redis =
    (pkgsHostTarget.redis.overrideAttrs (_: { doCheck = false; })).override {
      withSystemd = false;
    };
  schbench = callPackage ./schbench.nix { };
  sqlite-speedtest1 = callPackage ./sqlite-speedtest1.nix { };
  sysbench = if hostPlatform.isx86_64 then pkgsHostTarget.sysbench else null;

  # Pairs a package with the shell that installs its files into `$out`.
  packWith = pkg: install: { inherit pkg install; };

  benchmarkGroups = {
    # boot measures boot latency and needs no tool binaries, only its scripts
    # plus size padding. The case name "boot/boot_lat_<N>mb" encodes the pad to
    # add (N MiB, not the final image size); other boot cases add none. The pad
    # is a fixed-key AES-CTR keystream: deterministic (so the build stays
    # reproducible and cacheable) yet incompressible, so both the gzip image and
    # the unpacked rootfs grow by ~N MiB rather than being squeezed away by gzip.
    boot = packWith null (_:
      let m = builtins.match "boot/boot_lat_([0-9]+)mb" benchmarkName;
      in lib.optionalString (m != null) ''
        dd if=/dev/zero bs=1M count=${builtins.head m} status=none \
          | ${pkgsBuildBuild.openssl}/bin/openssl enc -aes-256-ctr -nosalt \
              -K 00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff \
              -iv 000102030405060708090a0b0c0d0e0f \
          > $out/pad.bin
      '');
    fio = packWith fio (p: "cp -r ${p}/bin/fio $out/bin/");
    hackbench = packWith hackbench (p: "cp -r ${p}/bin/hackbench $out/bin/");
    iperf3 = packWith iperf3 (p: "cp -r ${p}/bin/iperf3 $out/bin/");
    lmbench = packWith lmbench (p: ''
      mkdir -p $out/bin/lmbench
      cp -r ${p}/bin/* $out/bin/lmbench/
    '');
    memcached = packWith memcached (p: "cp -r ${p}/bin/memcached $out/bin/");
    nginx = packWith nginx (p: ''
      cp -r ${p}/bin/nginx $out/bin/
      mkdir -p $out/nginx/conf
      cp -r ${p}/conf/* $out/nginx/conf/
    '');
    redis = packWith redis (p: "cp -r ${p}/bin/redis-server $out/bin/");
    schbench = packWith schbench (p: "cp -r ${p}/bin/schbench $out/bin/");
    sqlite = packWith sqlite-speedtest1
      (p: "cp -r ${p}/bin/sqlite-speedtest1 $out/bin/");
  } // lib.optionalAttrs (sysbench != null) {
    sysbench = packWith sysbench (p: "cp -r ${p}/bin/sysbench $out/bin/");
  };

  # `benchmarkName` is the "<group>/<case>" selector (e.g. "sysbench/cpu_lat");
  # packaging is per group, so take the group here.
  benchmarkGroup = if benchmarkName == "none" then
    "none"
  else
    builtins.head (lib.splitString "/" benchmarkName);

  installSelected = group:
    if group == "none" then
      ""
    else if benchmarkGroups ? ${group} then
      let g = benchmarkGroups.${group}; in g.install g.pkg
    else
      throw ("Unknown benchmark group '${group}' (from '${benchmarkName}').");

  package = stdenvNoCC.mkDerivation {
    pname = "benchmark";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./../../src/benchmark;
      fileset = ./../../src/benchmark;
    };

    buildCommand = ''
      mkdir -p $out/bin

      # Pack the common files shared by every group.
      cp $src/bench_linux_and_aster.sh $out/
      cp -r $src/common $out/

      # Install the selected group's package.
      ${installSelected benchmarkGroup}

      # Pack the selected group's scripts.
      ${lib.optionalString (benchmarkGroup != "none")
      "cp -r $src/${benchmarkGroup} $out/"}
    '';
  };
}
