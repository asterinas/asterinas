{ disable-systemd ? "false", stage-2-hook ? "/bin/sh -l", log-level ? "error"
, console ? "hvc0", extra-substituters ? "", extra-trusted-public-keys ? ""
, config-file-name ? "configuration.nix", target_platform ? "x86_64-linux"
, pkgs ? import <nixpkgs> { } }:
let
  asterinas = builtins.path {
    name = "asterinas-osdk-bin";
    path = ../../target/osdk/iso_root/boot/asterinas-osdk-bin;
  };
  etc-nixos = builtins.path { path = ../etc_nixos; };

  aster_configuration = pkgs.replaceVarsWith {
    src = ./templates/aster_configuration.nix;
    replacements = {
      asterinas = asterinas;
      aster-disable-systemd = disable-systemd;
      aster-stage-2-hook = stage-2-hook;
      aster-log-level = log-level;
      aster-console = console;
      aster-target-platform = target_platform;
      aster-substituters = extra-substituters;
      aster-trusted-public-keys = extra-trusted-public-keys;
    };
  };
  aster_nixos_install = pkgs.replaceVarsWith {
    src = ./templates/aster-nixos-install;
    replacements = {
      aster-configuration = aster_configuration;
      aster-etc-nixos = etc-nixos;
      aster-target-platform = target_platform;
      aster-substituters = extra-substituters;
      aster-trusted-public-keys = extra-trusted-public-keys;
    };
    isExecutable = true;
  };

in pkgs.stdenv.mkDerivation {
  name = "aster_nixos_installer";
  buildCommand = ''
    mkdir -p $out/{bin,etc_nixos}
    install -m 755 ${aster_nixos_install} $out/bin/aster-nixos-install
    cp -L ${aster_configuration} $out/etc_nixos/aster_configuration.nix
    cp -L ${etc-nixos}/${config-file-name} $out/etc_nixos/configuration.nix
    cp -r ${etc-nixos}/modules $out/etc_nixos/modules
    cp -r ${etc-nixos}/overlays $out/etc_nixos/overlays
    ln -s ${asterinas} $out/kernel
  '';
}

