{ lib, stdenvNoCC, callPackage, hostPlatform, pkgsHostTarget, }: rec {
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

  package = stdenvNoCC.mkDerivation {
    pname = "benchmark";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./../../src;
      fileset = ./../../src/benchmark;
    };

    buildCommand = ''
      mkdir -p $out/bin
      cp -r ${fio}/bin/fio $out/bin/
      cp -r ${hackbench}/bin/hackbench $out/bin/
      cp -r ${iperf3}/bin/iperf3 $out/bin/
      cp -r ${memcached}/bin/memcached $out/bin/
      cp -r ${nginx}/bin/nginx $out/bin/
      cp -r ${redis}/bin/redis-server $out/bin/
      cp -r ${schbench}/bin/schbench $out/bin/
      cp -r ${sqlite-speedtest1}/bin/sqlite-speedtest1 $out/bin/

      mkdir -p $out/bin/lmbench
      cp -r ${lmbench}/bin/* $out/bin/lmbench/

      mkdir -p $out/nginx/conf
      cp -r ${nginx}/conf/* $out/nginx/conf/

      ${lib.optionalString (sysbench != null) ''
        cp -r ${sysbench}/bin/sysbench $out/bin/
      ''}

      cp -r $src/benchmark/* $out/
    '';
  };
}
