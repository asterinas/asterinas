{ lib, stdenv, fetchgit, pkgsBuildBuild, pkgsBuildHost, }:
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

  # Explicit allowlist of selftest subsystems Asterinas exercises today.
  # Keeping this small (rather than building "all") means
  # a regression in an unlisted subsystem upstream cannot silently change our conformance surface.
  # To enable a new subsystem,
  # add it here and triage any resulting build/run failures before landing.
  baseKselftestTargets =
    [ "exec" "lsm" "proc" "signal" "splice" "timers" "vDSO" ];

  kselftestTargets = lib.concatStringsSep " " (baseKselftestTargets
    ++ lib.optionals stdenv.hostPlatform.isx86_64 [ "x86" ]);

  enableParallelBuilding = true;

  nativeBuildInputs = with pkgsBuildBuild; [ gcc_multi rsync ];

  buildInputs = with pkgsBuildHost; [
    glibc_multi
    glibc_multi.static
    libcap.dev
  ];

  buildPhase = ''
    runHook preBuild
    make ARCH=${stdenv.hostPlatform.linuxArch} -j$NIX_BUILD_CORES headers
    make -C tools/testing/selftests ARCH=${stdenv.hostPlatform.linuxArch} \
         -j$NIX_BUILD_CORES TARGETS="$kselftestTargets" all
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    make -C tools/testing/selftests ARCH=${stdenv.hostPlatform.linuxArch} \
         -j$NIX_BUILD_CORES TARGETS="$kselftestTargets" KSFT_INSTALL_PATH=$out install
    runHook postInstall
  '';

  meta = with lib; {
    description =
      "Linux in-kernel selftests (kselftest) for Asterinas conformance testing";
    homepage = "https://docs.kernel.org/dev-tools/kselftest.html";
    license = licenses.gpl2Only;
    platforms = platforms.linux;
  };
}
