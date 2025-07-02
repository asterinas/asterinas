{ target ? "x86_64", enableBenchmark ? false, enableSyscallTest ? false, }:
let
  crossSystem.config = if target == "x86_64" then
    "x86_64-unknown-linux-gnu"
  else if target == "riscv64" then
    "riscv64-unknown-linux-gnu"
  else
    throw "Target arch ${target} not yet supported.";

  nixpkgs = fetchTarball "https://github.com/NixOS/nixpkgs/tarball/nixos-25.05";
  pkgs = import nixpkgs {
    config = { };
    overlays = [ ];
    inherit crossSystem;
  };
in rec {
  # Packages needed by initramfs
  busybox = pkgs.busybox;
  benchmark = pkgs.callPackage ./benchmark { };
  syscall = pkgs.callPackage ./syscall { };
  linux_vdso = pkgs.fetchFromGitHub {
    owner = "asterinas";
    repo = "linux_vdso";
    rev = "be255018febf8b9e2d36f356f6aeb15896521618";
    hash = "sha256-F5RPtu/Hh2hDnjm6/0mc0wGqhQtfMNvPP+6/Id9Hcpk";
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
