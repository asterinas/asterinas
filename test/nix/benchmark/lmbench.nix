{ stdenv, fetchFromGitHub, libtirpc, }:
stdenv.mkDerivation {
  pname = "lmbench";
  version = "0.1.0";
  src = fetchFromGitHub {
    owner = "asterinas";
    repo = "lmbench";
    rev = "25a43f544af396b81c7a378c83d33f2cbab10fcc";
    hash = "sha256-HGhBNuR5rrSAsk6c2bD0YuVV+5w7itCNVVxFRD522Rw";
  };

  dontPatchShebangs = true;
  makeFlags = [ "CC=${stdenv.cc.targetPrefix}cc" ];
  patchPhase = ''
    runHook prePatch

    substituteInPlace src/Makefile \
      --replace-fail "/bin/rm" "rm" \
      --replace-fail "AR=ar" ""

    runHook postPatch
  '';
  buildInputs = [ libtirpc ];
  preBuild = ''
    makeFlagsArray+=(CPPFLAGS="-std=gnu89 -I${libtirpc.dev}/include/tirpc -Wno-error=format-security")
  '';
  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin
    mv bin/x86_64-linux-gnu/* $out/bin/

    runHook postInstall
  '';
}
