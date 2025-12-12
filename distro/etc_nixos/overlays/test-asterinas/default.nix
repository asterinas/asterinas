final: prev: {
  test-asterinas = prev.stdenv.mkDerivation {
    name = "test-asterinas";
    version = "0.1.0";

    src = ./.;

    installPhase = ''
      install -m755 -D $src/test-nix-commands.sh $out/bin/test-nix-commands
      install -m755 -D $src/test-podman.sh $out/bin/test-podman
    '';
  };
}
