{
  lib,
  stdenv,
  fetchFromGitHub,
  pkgs,
  pkgsBuildBuild,
  conformanceSrc,
}:

let
  runtimeDeps = with pkgs; [
    perl
    coreutils
    gnugrep
    gnused
    gawk
    openssl
  ];

  runtimePath = lib.makeBinPath runtimeDeps + ":/bin:/usr/bin";
in
stdenv.mkDerivation rec {
  pname = "pjdfstest";
  version = "0.1";

  src = fetchFromGitHub {
    owner = "pjd";
    repo = "pjdfstest";
    rev = "85a8aea9e685999ef0540392fd80535f873d7ff7";
    hash = "sha256-tYF5D2JAvDn/a6y52EVHwzg8iRtZ72g84iDJgqK+xBw=";
  };

  nativeBuildInputs = with pkgsBuildBuild; [
    autoreconfHook
    pkg-config
  ];

  enableParallelBuilding = true;

  # The fixup pass would rewrite the `#!/bin/sh` shebangs of the suite's
  # scripts to store paths that do not exist inside the guest, which would stop
  # `prove` from running them.
  dontPatchShebangs = true;

  stripDebugList = [ "pjdfstest" ];

  installPhase = ''
    runHook preInstall

    mkdir -p $out/tests
    install -m 00755 pjdfstest $out/pjdfstest

    # The upstream tarball ships `tests/` and the test cases themselves as
    # non-executable, but `prove` runs them directly through their shebangs.
    cp -r tests/. $out/tests/
    chmod +x $out/tests/*/*.t

    install -m 00644 ${conformanceSrc}/pjdfstest/conf $out/tests/conf

    for fs in ramfs ext2; do
      mkdir -p $out/$fs
      install -m 00644 ${conformanceSrc}/pjdfstest/$fs/runlist $out/$fs/runlist
      install -m 00644 ${conformanceSrc}/pjdfstest/$fs/blocklist $out/$fs/blocklist
    done
    install -m 00755 ${conformanceSrc}/pjdfstest/run_pjdfstest_test.sh $out/run_pjdfstest_test.sh
    substituteInPlace $out/run_pjdfstest_test.sh \
      --replace-fail '__RUNTIME_PATH__' '${runtimePath}'

    runHook postInstall
  '';
}
