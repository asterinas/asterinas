{ lib
, stdenv
, fetchFromGitHub
, libtirpc
, makeWrapper
}:
stdenv.mkDerivation rec {
  pname = "lmbench";
  version = "1.0.0";

  src = fetchFromGitHub {
    owner = "asterinas";
    repo = "lmbench";
    rev = "31f49994d206c9f5a781741e1ca4b300dc70a631";
    sha256 = "sha256-WvH+VuSfnffhvU0ajJtKE9Nlkaqb1L3nvGJT+n6QPIc=";
  };

  postPatch = ''
    substituteInPlace src/Makefile \
      --replace-fail '/bin/rm' 'rm'
    substituteInPlace src/Makefile \
      --replace-fail 'AR=ar' ""
  '';

  buildInputs = [ libtirpc ];
  nativeBuildInputs = [ makeWrapper ];

  enableParallelBuilding = true;

  CPPFLAGS = "-I${libtirpc.dev}/include/tirpc -Wno-error=implicit-function-declaration -Wno-error=implicit-int -Wno-error=return-type -Wno-error=format-security";
  LDFLAGS = "-ltirpc -lkrb5 -lkrb5support -lk5crypto -lcom_err -lgssrpc -lgssapi_krb5";

  buildPhase = let
    prefix = "${stdenv.cc}/bin/${stdenv.cc.targetPrefix}";
  in ''
    make CC=${prefix}cc AR=${prefix}ar -j
  '';

  installPhase = ''
    mkdir -p $out/bin
    cp ./bin/x86_64-linux-gnu/* $out/bin/
  '';
}
