{ disable-systemd ? "false", stage-2-hook ? "/bin/sh -l", log-level ? "error"
, console ? "hvc0", extra-substituters ? "", extra-trusted-public-keys ? ""
, config-file-name ? "configuration.nix", pkgs ? import <nixpkgs> { } }:
let
  aster-kernel = builtins.path {
    name = "aster-kernel-osdk-bin";
    path = ../../target/osdk/iso_root/boot/aster-kernel-osdk-bin;
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
      aster-substituters = extra-substituters;
      aster-trusted-public-keys = extra-trusted-public-keys;
    };
  };
  install_aster_nixos = pkgs.replaceVarsWith {
    src = ./templates/install_nixos.sh;
    replacements = {
      aster-configuration = aster_configuration;
      aster-etc-nixos = etc-nixos;
      aster-substituters = extra-substituters;
      aster-trusted-public-keys = extra-trusted-public-keys;
    };
    isExecutable = true;
  };

in pkgs.stdenv.mkDerivation {
  name = "aster_nixos_installer";
  buildCommand = ''
    mkdir -p $out/{bin,etc_nixos}
    cp ${install_aster_nixos} $out/bin/install_aster_nixos.sh
    cp -L ${aster_configuration} $out/etc_nixos/aster_configuration.nix
    cp -L ${etc-nixos}/${config-file-name} $out/etc_nixos/configuration.nix
    cp -r ${etc-nixos}/modules $out/etc_nixos/modules
    cp -r ${etc-nixos}/overlays $out/etc_nixos/overlays
    ln -s ${aster-kernel} $out/kernel
  '';
}

