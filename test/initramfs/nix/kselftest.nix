{ lib, stdenv, fetchFromGitHub, replaceVarsWith, pkgsBuildBuild, pkgsBuildHost,
}: rec {
  kselftest = stdenv.mkDerivation rec {
    pname = "kselftest-bin";
    version = "6.18";
    src = fetchFromGitHub {
      owner = "torvalds";
      repo = "linux";
      tag = "v${version}";
      hash = "sha256-F1vg95nMGiXk9zbUzg+/hUq+RjXdFmtN530b7QuqkMc";
    };

    nativeBuildInputs = with pkgsBuildBuild; [
      bison
      flex
      gcc_multi
      rsync
      pkg-config
      python312
      python312Packages.pyyaml
      python312Packages.jsonschema
    ];

    buildInputs = with pkgsBuildHost; [
      alsa-lib.dev
      elfutils.dev
      fuse.dev
      glibc_multi
      glibc_multi.static
      libcap.dev
      libcap_ng.dev
      libelf
      libmnl
      libnl.dev
      liburing.dev
      mbedtls
      numactl.dev
      openssl.dev
      popt
      zlib.dev
    ];

    configurePhase = ''
      runHook preConfigure

      patchShebangs tools/net/ynl/pyynl
      sed -i '206d' tools/testing/selftests/net/Makefile # FIXME: bpf build fails
      sed -i '17d' tools/testing/selftests/cgroup/Makefile # FIXME: test_memcontrol build fails
      make defconfig

      runHook postConfigure
    '';

    buildPhase = ''
      runHook preBuild

      make kselftest-all

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      make KSFT_INSTALL_PATH=$out kselftest-install

      runHook postInstall
    '';
  };

  run_kselftest = replaceVarsWith {
    src = ./../src/kselftest/run_kselftest.sh;
    replacements = { inherit kselftest; };
    isExecutable = true;
  };

  package = stdenv.mkDerivation {
    pname = "kselftest";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./../src/kselftest;
      fileset = ./../src/kselftest;
    };
    buildCommand = ''
      mkdir -p $out/kselftest
      cp -r $src/blocklists $out/kselftest
      cp ${run_kselftest} $out/kselftest/run_kselftest.sh
    '';
  };
}
