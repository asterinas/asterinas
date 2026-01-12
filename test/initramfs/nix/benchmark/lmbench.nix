{ stdenv, fetchFromGitHub, libtirpc, }:
stdenv.mkDerivation {
  pname = "lmbench";
  version = "0.1.0";
  src = fetchFromGitHub {
    owner = "asterinas";
    repo = "lmbench";
    rev = "afb47eddaf10a411c1ea3cb64965461f1308a6ea";
    hash = "sha256-XFcvWwqOci1jdNaBGj7/svCT49JAppYvN+ABeO9rqbw";
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
