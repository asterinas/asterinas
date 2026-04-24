{ lib, stdenvNoCC, callPackage, testSuite ? "ltp", workDir ? "/tmp", smp ? 1,
}: rec {
  inherit testSuite;
  ltp = callPackage ./ltp.nix { };
  # FIXME: Build gvisor syscall test with nix.
  gvisor = builtins.path {
    name = "gvisor-prebuilt";
    path = builtins.getEnv "GVISOR_PREBUILT_DIR";
  };
  kselftest = callPackage ./kselftest.nix { };

  package = stdenvNoCC.mkDerivation {
    pname = "conformance";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./../../src/conformance;
      fileset = ./../../src/conformance;
    };
    buildCommand = ''
      cd $src
      mkdir -p $out
      export INITRAMFS=$out
      export CONFORMANCE_TEST_SUITE=${testSuite}
      export CONFORMANCE_TEST_WORKDIR=${workDir}
      export SMP=${toString smp}
      ${lib.optionalString (testSuite == "ltp")
      "export LTP_PREBUILT_DIR=${ltp}"}
      ${lib.optionalString (testSuite == "gvisor")
      "export GVISOR_PREBUILT_DIR=${gvisor}"}
      ${lib.optionalString (testSuite == "kselftest")
      "export KSELFTEST_PREBUILT_DIR=${kselftest}"}
      make
    '';
  };
}
