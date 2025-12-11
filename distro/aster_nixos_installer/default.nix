{ disable-systemd ? "false", stage-2-hook ? "/bin/sh -l", log-level ? "error"
, console ? "hvc0", test-command ? "", pkgs ? import <nixpkgs> { } }:
let
  aster-kernel = builtins.path {
    name = "aster-nix-osdk-bin";
    path = ../../target/osdk/iso_root/boot/aster-nix-osdk-bin;
  };
  etc-nixos = builtins.path { path = ../etc_nixos; };

  aster_configuration = pkgs.replaceVarsWith {
    src = ./templates/aster_configuration.nix;
    replacements = {
      aster-kernel = aster-kernel;
      aster-disable-systemd = disable-systemd;
      aster-stage-2-hook = stage-2-hook;
      aster-log-level = log-level;
      aster-console = console;
      aster-test-command = test-command;
    };
  };
  install_aster_nixos = pkgs.replaceVarsWith {
    src = ./templates/install_nixos.sh;
    replacements = {
      aster-configuration = aster_configuration;
      aster-etc-nixos = etc-nixos;
    };
    isExecutable = true;
  };

in pkgs.stdenv.mkDerivation {
  name = "aster_nixos_installer";
  buildCommand = ''
    mkdir -p $out/{bin,etc_nixos}
    cp ${install_aster_nixos} $out/bin/install_aster_nixos.sh
    ln -s ${aster_configuration} $out/etc_nixos/aster_configuration.nix
    ln -s ${etc-nixos}/configuration.nix $out/etc_nixos/configuration.nix
    ln -s ${etc-nixos}/modules $out/etc_nixos/modules
    ln -s ${etc-nixos}/overlays $out/etc_nixos/overlays
    ln -s ${aster-kernel} $out/kernel
  '';
}

