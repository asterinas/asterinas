{ lib, stdenv, fetchFromGitHub, pkgsBuildBuild, pkgsBuildHost, }:
stdenv.mkDerivation rec {
  pname = "kselftest";
  version = "6.18";
  src = fetchFromGitHub {
    owner = "torvalds";
    repo = "linux";
    tag = "v${version}";
    hash = "sha256-F1vg95nMGiXk9zbUzg+/hUq+RjXdFmtN530b7QuqkMc";
  };

  patches = [ ./0001-Skip-build-targets-that-would-fail.patch ];

  nativeBuildInputs = with pkgsBuildBuild; [
    bison
    flex
    gcc_multi
    rsync
    pkg-config
    (python312.withPackages (p: with p; [ jsonschema pyyaml ]))
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
}
