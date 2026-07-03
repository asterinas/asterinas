{ stdenv, fetchFromGitHub, hostPlatform, libcap, pkgsBuildBuild, python3Minimal,
}:
stdenv.mkDerivation rec {
  pname = "ltp";
  version = "20260529";
  src = fetchFromGitHub {
    owner = "linux-test-project";
    repo = "ltp";
    rev = "${version}";
    hash = "sha256-h4cIK0sDbyGiGwvba3jkJ+W28oNzvQAfGu1RbGAFljA=";
  };
  # Kirk is the official LTP tests executor.
  kirkSrc = fetchFromGitHub {
    owner = "linux-test-project";
    repo = "kirk";
    # Pinned to version v4.1.0, which is the exact version of the kirk
    # submodule specified by the LTP 20260529 release.
    rev = "v4.1.0";
    hash = "sha256-W/6sxdqNACs0yI+g8BLYFdqPzJXA1VHYbUN/YL7kuJE=";
  };

  # Clear `CFLAGS` and `DEBUG_CFLAGS` to prevent `-g` from being automatically added.
  CFLAGS = "";
  DEBUG_CFLAGS = "";
  dontPatchShebangs = true;
  enableParallelBuilding = true;
  nativeBuildInputs = with pkgsBuildBuild; [
    automake
    autoconf
    libtool
    gnum4
    makeWrapper
    pkg-config
  ];
  buildInputs = [ libcap ];
  configurePhase = ''
    runHook preConfigure

    make autotools
    ./configure --host ${hostPlatform.system} --prefix=$out

    runHook postConfigure
  '';
  buildPhase = ''
    runHook preBuild

    make -C testcases/kernel
    make -C testcases/lib
    make -C runtest

    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall

    make -C testcases/kernel install
    make -C testcases/lib install
    make -C runtest install

    cp -r ${kirkSrc}/libkirk $out/libkirk
    install -m 00755 ${kirkSrc}/kirk $out/kirk
    substituteInPlace $out/kirk \
      --replace-fail '#!/usr/bin/env python3' '#!${python3Minimal}/bin/python3'
    install -m 00444 $src/VERSION $out/Version
    install -m 00755 $src/ver_linux $out/ver_linux
    install -m 00755 $src/IDcheck.sh $out/IDcheck.sh

    runHook postInstall
  '';
}
