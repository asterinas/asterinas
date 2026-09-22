{
  stdenv,
  pkg-config,
  libdrm,
}:
stdenv.mkDerivation {
  pname = "drm-tools";
  version = "0.1.0";

  src = ../../../tools/drm;

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ libdrm ];

  installPhase = ''
    runHook preInstall
    make install PREFIX="$out"
    runHook postInstall
  '';
}
