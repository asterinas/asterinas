{ target ? "x86_64", enableBenchmarkTest ? false, enableConformanceTest ? false
, enableRegressionTest ? false, conformanceTestSuite ? "ltp"
, conformanceTestWorkDir ? "/tmp", regressionTestPlatform ? "asterinas"
, dnsServer ? "none", smp ? 1, initramfsCompressed ? true, }:
let
  crossSystem.config = if target == "x86_64" then
    "x86_64-unknown-linux-gnu"
  else if target == "riscv64" then
    "riscv64-unknown-linux-gnu"
  else
    throw "Target arch ${target} not yet supported.";

  # Pinned nixpkgs (nix version: 2.34.7, channel: nixos-26.05)
  nixpkgs = fetchTarball {
    url =
      "https://github.com/NixOS/nixpkgs/archive/fd1462031fdee08f65fd0b4c6b64e22239a77870.tar.gz";
    sha256 = "0h0snjjawavy0gl176iyxqdcmv85vx3nlm0aalwr1q8m2960ly4z";
  };
  pkgs = import nixpkgs {
    config = { };
    overlays = [ ];
    inherit crossSystem;
  };
in rec {
  # Packages needed by initramfs
  busybox = pkgs.busybox;
  benchmark = pkgs.callPackage ./benchmark { };
  conformance = pkgs.callPackage ./conformance {
    inherit smp;
    testSuite = conformanceTestSuite;
    workDir = conformanceTestWorkDir;
  };
  regression =
    pkgs.callPackage ./regression { testPlatform = regressionTestPlatform; };

  initramfs = pkgs.callPackage ./initramfs.nix {
    inherit busybox;
    benchmark = if enableBenchmarkTest then benchmark else null;
    conformance = if enableConformanceTest then conformance else null;
    regression = if enableRegressionTest then regression else null;
    dnsServer = dnsServer;
  };
  initramfs-image = pkgs.callPackage ./initramfs-image.nix {
    inherit initramfs;
    compressed = initramfsCompressed;
  };

  # Packages needed by host
  apacheHttpd = pkgs.apacheHttpd;
  iperf3 = pkgs.iperf3;
  libmemcached = pkgs.libmemcached.overrideAttrs (_: {
    configureFlags = [ "--enable-memaslap" ];
    LDFLAGS = "-lpthread";
    CPPFLAGS = "-fcommon -fpermissive";
  });
  lmbench = pkgs.callPackage ./benchmark/lmbench.nix { };
  redis = (pkgs.redis.overrideAttrs (_: { doCheck = false; })).override {
    withSystemd = false;
  };
}
