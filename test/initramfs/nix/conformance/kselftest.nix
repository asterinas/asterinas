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
      "tools/build"
      "tools/lib"
      "arch"
      "include"
      "scripts"
      "samples/check-exec"
    ];
    hash = "sha256-kL6i8EhUYE600eLZ9G2PbCHaD9GQJsYuRnvkq+N8FUA=";
  };

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
      "kvm"
    ]
  );

  enableParallelBuilding = true;

  nativeBuildInputs = with pkgsBuildBuild; [ rsync ];

  buildInputs = with pkgs; [
    glibc_multi.static
    libcap.dev
  ];

  postPatch = ''
    substituteInPlace tools/testing/selftests/kvm/Makefile \
      --replace-fail 'ARCH := x86' 'override ARCH := x86'
  '';

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
    runHook postInstall
  '';

  meta = with lib; {
    description = "Linux in-kernel selftests (kselftest) for Asterinas conformance testing";
    homepage = "https://docs.kernel.org/dev-tools/kselftest.html";
    license = licenses.gpl2Only;
    platforms = platforms.linux;
  };
}
