{
  target ? "x86_64",
  enableBenchmarkTest ? false,
  enableConformanceTest ? false,
  enableRegressionTest ? false,
  conformanceTestSuite ? "ltp",
  conformanceTestWorkDir ? "/tmp",
  conformanceTestSelector ? "",
  regressionTestPlatform ? "asterinas",
  dnsServer ? "none",
  smp ? 1,
  initramfsCompressed ? true,
  benchmarkName ? "none",
  system ? builtins.currentSystem,
}:
let
  crossSystem.config =
    if target == "x86_64" then
      "x86_64-unknown-linux-gnu"
    else if target == "riscv64" then
      "riscv64-unknown-linux-gnu"
    else if target == "aarch64" then
      "aarch64-unknown-linux-gnu"
    else
      throw "Target arch ${target} not yet supported.";

  pkgs = import ../../../distro/nixpkgs.nix {
    config = { };
    overlays = [ ];
    inherit system crossSystem;
  };
  buildMachinePkgs = pkgs.pkgsBuildBuild;
in
rec {
  # Packages needed by initramfs
  busybox = pkgs.busybox;
  benchmark = pkgs.callPackage ./benchmark { inherit benchmarkName; };
  conformance = pkgs.callPackage ./conformance {
    inherit smp;
    testSuite = conformanceTestSuite;
    workDir = conformanceTestWorkDir;
    testSelector = conformanceTestSelector;
  };
  regression = pkgs.callPackage ./regression { testPlatform = regressionTestPlatform; };

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
  rootfs-image = pkgs.callPackage ./rootfs-image.nix { inherit initramfs; };

  # These clients run on the build machine, so use its native packages here.
  # This only selects host clients; guest packages above still use `pkgs` and
  # keep the requested guest architecture. Use `pkgsBuildBuild` because lmbench
  # reads `targetPlatform` for its build flags and installation paths;
  # `buildPackages` would leave that field tied to the guest architecture.
  apacheHttpd = buildMachinePkgs.apacheHttpd;
  iperf3 = buildMachinePkgs.iperf3;
  libmemcached = buildMachinePkgs.libmemcached.overrideAttrs (_: {
    configureFlags = [ "--enable-memaslap" ];
    LDFLAGS = "-lpthread";
    CPPFLAGS = "-fcommon -fpermissive";
  });
  lmbench = buildMachinePkgs.callPackage ./benchmark/lmbench.nix { };
  redis =
    (buildMachinePkgs.redis.overrideAttrs (_: {
      doCheck = false;
    })).override
      { withSystemd = false; };
}
