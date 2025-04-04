{ lib, stdenv, callPackage, hostPlatform, pkgsHostTarget, }:
let
  # Define the PARSEC kernel and app lists
  parsecKernels = [ "canneal" "streamcluster" "dedup" ];
  parsecApps = [
    "blackscholes"
    "bodytrack"
    "facesim"
    "ferret"
    "fluidanimate"
    "freqmine"
    "vips"
    "x264"
  ];

  # Create a package for the mm-scalability-benchmark binaries
  benchmarkSrc = /root/mm-scalability-benchmark;
  pathExists = builtins.pathExists benchmarkSrc;
in rec {
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

  package = stdenv.mkDerivation {
    pname = "benchmark";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./../../src;
      fileset = ./../../src/benchmark;
    };

    buildCommand = ''
      mkdir -p $out/bin
      # cp -r ${fio}/bin/fio $out/bin/
      # cp -r ${hackbench}/bin/hackbench $out/bin/
      # cp -r ${iperf3}/bin/iperf3 $out/bin/
      # cp -r ${memcached}/bin/memcached $out/bin/
      # cp -r ${nginx}/bin/nginx $out/bin/
      # cp -r ${redis}/bin/redis-server $out/bin/
      # cp -r ${schbench}/bin/schbench $out/bin/
      # cp -r ${sqlite-speedtest1}/bin/sqlite-speedtest1 $out/bin/

      mkdir -p $out/bin/lmbench
      cp -r ${lmbench}/bin/* $out/bin/lmbench/

      # mkdir -p $out/nginx/conf
      # cp -r ${nginx}/conf/* $out/nginx/conf/

      ${lib.optionalString (sysbench != null) ''
        cp -r ${sysbench}/bin/sysbench $out/bin/
      ''}

      mkdir -p $out/bin/vm_scale_bench_data

      # Copy metis
      mkdir -p $out/bin/metis
      cp ${benchmarkSrc}/mosbench/metis_with_radixvm/metis/obj/app/wrmem $out/bin/metis/

      # Copy psearchy
      mkdir -p $out/bin/psearchy
      cp ${benchmarkSrc}/mosbench/psearchy/mkdb/pedsort $out/bin/psearchy/
      cp ${benchmarkSrc}/mosbench/psearchy/mkdb/pedsort-tc $out/bin/psearchy/
      cp ${benchmarkSrc}/mosbench/psearchy/linux_files_index $out/bin/psearchy/

      # Copy PARSEC kernels
      ${lib.concatMapStringsSep "\n" (kernel: ''
        mkdir -p $out/bin/${kernel}
        cp ${benchmarkSrc}/parsec-3.0/pkgs/kernels/${kernel}/inst/amd64-linux.gcc/bin/${kernel} $out/bin/${kernel}/
      '') parsecKernels}

      # Copy PARSEC apps
      ${lib.concatMapStringsSep "\n" (app: ''
        mkdir -p $out/bin/${app}
        cp ${benchmarkSrc}/parsec-3.0/pkgs/apps/${app}/inst/amd64-linux.gcc/bin/${app} $out/bin/${app}/
      '') parsecApps}

      # Copy swaptions (special case)
      mkdir -p $out/bin/swaptions
      cp ${benchmarkSrc}/parsec-3.0/pkgs/apps/swaptions/obj/amd64-linux.gcc-tbb/swaptions $out/bin/swaptions/

      # Copy dedup-tc
      mkdir -p $out/bin/dedup
      cp ${benchmarkSrc}/dedup-tc $out/bin/dedup/

      # Create vm_scale_bench_data directory
      mkdir -p $out/bin/vm_scale_bench_data
    '';
  };
}
