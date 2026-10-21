{
  lib,
  stdenv,
  fetchFromGitHub,
}:
stdenv.mkDerivation {
  pname = "kvm-hello-world";
  version = "unstable-2018-08-06";

  src = fetchFromGitHub {
    owner = "dpw";
    repo = "kvm-hello-world";
    rev = "e9ab0f26e892fa3794f2f991144be7fa45ddd082";
    hash = "sha256-xHKC1qqDO5IVbXke2tF9zMEOzY7Twalg1NX6kxZH0Xk=";
  };

  # Guest payloads run without a C runtime or position-independent code.
  hardeningDisable = [
    "pic"
    "stackprotector"
  ];

  postPatch = ''
    # Keep payload.o relocatable for the final executable link.
    substituteInPlace Makefile \
      --replace-fail '$(LD) -T $< -o $@' '$(LD) -r -T $< -o $@'
  '';

  buildPhase = ''
    runHook preBuild
    # The default target runs the program; only build it here.
    make kvm-hello-world \
      CC=${stdenv.cc.targetPrefix}cc \
      LD=${stdenv.cc.targetPrefix}ld AS=${stdenv.cc.targetPrefix}as \
      CFLAGS="-Wall -Wextra -Werror -Wno-array-bounds -O2"
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm755 kvm-hello-world $out/bin/kvm-hello-world
    runHook postInstall
  '';

  meta = {
    description = "A minimal KVM userspace example";
    homepage = "https://github.com/dpw/kvm-hello-world";
    license = lib.licenses.mit;
    platforms = [ "x86_64-linux" ];
  };
}
