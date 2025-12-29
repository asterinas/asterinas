final: prev: {
  test-asterinas = prev.stdenv.mkDerivation {
    name = "test-asterinas";
    version = "0.1.0";

    src = ./.;

    installPhase = ''
      mkdir -p $out/bin
      cp $src/*.sh $out/bin/*.sh
      chmod +x $out/bin/*.sh
    '';
  };
}
