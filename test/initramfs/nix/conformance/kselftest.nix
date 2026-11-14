{
  lib,
  stdenv,
  fetchgit,
  pkgs,
  pkgsBuildBuild,
  pkgsBuildHost,
}:
let
  crossCompilePrefix = pkgsBuildHost.gcc.targetPrefix;
  hostCc = "${pkgsBuildBuild.gcc}/bin/gcc";
in
stdenv.mkDerivation rec {
  pname = "kselftest";
  version = "6.18";

  # Sparse-checkout the kselftest subtree instead of the full ~1 GB kernel tree.
  # To bump: update `version`, temporarily set `hash = lib.fakeHash;`, and run
  # the Nix build once to let `fetchgit` report the real hash for this sparse
  # checkout set.
  src = fetchgit {
    url = "https://github.com/torvalds/linux.git";
    rev = "v${version}";
    sparseCheckout = [
      "tools/testing/selftests"
      "tools/arch"
      "tools/include"
      "tools/scripts"
      "arch"
      "include"
      "scripts"
      "samples/check-exec"
    ];
    hash = "sha256-7Ie6f1aNvBo6ipoS/no+IkVhQ/C9bedzQCUoYwM4ED4=";
  };

  patches = [ ./kselftest-proc-pidns-include-fcntl.patch ];

  # Explicit allowlist of selftest subsystems Asterinas exercises today.
  # Keeping this small (rather than building "all") means
  # a regression in an unlisted subsystem upstream cannot silently change our conformance surface.
  # To enable a new subsystem,
  # add it here and triage any resulting build/run failures before landing.
  baseKselftestTargets = [
    "exec"
    "lsm"
    "proc"
    "signal"
    "splice"
    "timers"
    "vDSO"
  ];

  kselftestTargets = lib.concatStringsSep " " (
    baseKselftestTargets
    ++ lib.optionals stdenv.hostPlatform.isx86_64 [
      "x86"
    ]
  );

  enableParallelBuilding = true;

  nativeBuildInputs = with pkgsBuildBuild; [ rsync ];

  buildInputs =
    with pkgs;
    [ libcap.dev ]
    ++ lib.optionals stdenv.hostPlatform.isx86_64 [ glibc_multi.static ]
    ++ lib.optionals (!stdenv.hostPlatform.isx86_64) [
      glibc
      glibc.static
    ];

  buildPhase = ''
    runHook preBuild
    make ARCH=${stdenv.hostPlatform.linuxArch} \
         CROSS_COMPILE=${crossCompilePrefix} HOSTCC=${hostCc} \
         -j$NIX_BUILD_CORES headers
    make -C tools/testing/selftests ARCH=${stdenv.hostPlatform.linuxArch} \
         CROSS_COMPILE=${crossCompilePrefix} HOSTCC=${hostCc} \
         -j$NIX_BUILD_CORES TARGETS="$kselftestTargets" all
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    make -C tools/testing/selftests ARCH=${stdenv.hostPlatform.linuxArch} \
         CROSS_COMPILE=${crossCompilePrefix} HOSTCC=${hostCc} \
         -j$NIX_BUILD_CORES TARGETS="$kselftestTargets" KSFT_INSTALL_PATH=$out install

    for target in $kselftestTargets; do
      if [ ! -d "$out/$target" ] || ! grep -q "^$target:" "$out/kselftest-list.txt"; then
        echo "kselftest target was not installed completely: $target" >&2
        exit 1
      fi
    done
    runHook postInstall
  '';

  meta = with lib; {
    description = "Linux in-kernel selftests (kselftest) for Asterinas conformance testing";
    homepage = "https://docs.kernel.org/dev-tools/kselftest.html";
    license = licenses.gpl2Only;
    platforms = platforms.linux;
  };
}
