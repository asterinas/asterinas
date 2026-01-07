{ stdenv, fetchFromGitHub, hostPlatform, pkgsBuildBuild, }:
stdenv.mkDerivation rec {
  pname = "ltp";
  version = "v20250130";
  src = fetchFromGitHub {
    owner = "asterinas";
    repo = "ltp";
    rev = "${version}";
    hash = "sha256-cGT9Co8Fi3mL7oO+Fq2oMQDZDz5srrfyhkokPFTQUXc";
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
  configurePhase = ''
    runHook preConfigure

    make autotools
    ./configure --host ${hostPlatform.system} --prefix=$out

    runHook postConfigure
  '';
  buildPhase = ''
    runHook preBuild

    make -C testcases/kernel/syscalls
    make -C testcases/lib
    make -C runtest
    make -C pan

    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall

    make -C testcases/kernel/syscalls install
    make -C testcases/lib install
    make -C runtest install
    make -C pan install

    install -m 00755 $src/runltp $out/runltp
    install -m 00444 $src/VERSION $out/Version
    install -m 00755 $src/ver_linux $out/ver_linux
    install -m 00755 $src/IDcheck.sh $out/IDcheck.sh

    runHook postInstall
  '';
}
