{ stdenv, fetchFromGitHub, pkgsBuildBuild, }:
stdenv.mkDerivation rec {
  pname = "sqlite-speedtest1";
  version = "3.48.0";
  src = fetchFromGitHub {
    owner = "sqlite";
    repo = "sqlite";
    rev = "version-${version}";
    sha256 = "sha256-/qC1Jt+HFAwx4GTyOPCRQSn/ORZ9qmpTX0HhU+R5oWg";
  };

  configureFlags = [ "--enable-all" ];
  nativeBuildInputs = [ pkgsBuildBuild.gcc ];
  buildPhase = ''
    runHook preBuild

    make speedtest1

    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin
    mv speedtest1 $out/bin/sqlite-speedtest1

    runHook postInstall
  '';
}
