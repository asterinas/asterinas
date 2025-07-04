{ lib, stdenv, callPackage, testSuite ? "ltp", workDir ? "/tmp", smp ? 1,
}: rec {
  inherit testSuite;
  ltp = callPackage ./ltp.nix { };
  # FIXME: Build gvisor syscall test with nix.
  gvisor = builtins.path {
    name = "gvisor-prebuilt";
    path = builtins.getEnv "GVISOR_PREBUILT_DIR";
  };

  package = stdenv.mkDerivation {
    pname = "syscall_test";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./../../src;
      fileset = ./../../src/syscall;
    };
    buildCommand = ''
      cd $src/syscall
      mkdir -p $out
      export INITRAMFS=$out
      export LTP_PREBUILT_DIR=${ltp}
      export GVISOR_PREBUILT_DIR=${gvisor}
      export SYSCALL_TEST_SUITE=${testSuite}
      export SYSCALL_TEST_WORKDIR=${workDir}
      export SMP=${toString smp}
      make
    '';
  };
}
