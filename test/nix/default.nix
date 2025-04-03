{ target ? "x86_64", enableBasicTest ? false, enableBenchmark ? false
, enableSyscallTest ? false, syscallTestSuite ? "ltp"
, syscallTestWorkDir ? "/tmp", smp ? 1, initramfsCompressed ? true, }:
let
  crossSystem.config = if target == "x86_64" then
    "x86_64-unknown-linux-gnu"
  else if target == "riscv64" then
    "riscv64-unknown-linux-gnu"
  else
    throw "Target arch ${target} not yet supported.";

  # Pinned nixpkgs (nix version: 2.29.1, channel: nixos-25.05, release date: 2025-07-01)
  nixpkgs = fetchTarball {
    url =
      "https://github.com/NixOS/nixpkgs/archive/c0bebd16e69e631ac6e52d6eb439daba28ac50cd.tar.gz";
    sha256 = "1fbhkqm8cnsxszw4d4g0402vwsi75yazxkpfx3rdvln4n6s68saf";
  };
  pkgs = import nixpkgs {
    config = { };
    overlays = [ ];
    inherit crossSystem;
  };
in rec {
  # Packages needed by initramfs
  apps = pkgs.callPackage ./apps.nix { };
  busybox = pkgs.busybox;
  benchmark = pkgs.callPackage ./benchmark { };
  syscall = pkgs.callPackage ./syscall {
    inherit smp;
    testSuite = syscallTestSuite;
    workDir = syscallTestWorkDir;
  };
  linux_vdso = pkgs.fetchFromGitHub {
    owner = "asterinas";
    repo = "linux_vdso";
    rev = "be255018febf8b9e2d36f356f6aeb15896521618";
    hash = "sha256-F5RPtu/Hh2hDnjm6/0mc0wGqhQtfMNvPP+6/Id9Hcpk";
  };
  initramfs = pkgs.callPackage ./initramfs.nix {
    inherit busybox linux_vdso;
    apps = if enableBasicTest then apps else null;
    benchmark = if enableBenchmark then benchmark else null;
    syscall = if enableSyscallTest then syscall else null;
    # Add the required libraries for Metis/Dedup/Psearchy for CortenMM Eval
    db = pkgs.db.out;
    libgomp = pkgs.gcc.cc.lib; # libgomp is part of gcc
    gperftools = pkgs.gperftools;
    jdk21_headless = pkgs.jdk21_headless;
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
