{ pkgs ? import <nixpkgs> { }, autoInstall ? false, test-command ? ""
, extra-substituters ? "", extra-trusted-public-keys ? "", version ? "", ... }:
let
  installer = pkgs.callPackage ../aster_nixos_installer {
    inherit test-command extra-substituters extra-trusted-public-keys;
  };
  configuration = {
    imports = [
      "${pkgs.path}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
      "${pkgs.path}/nixos/modules/installer/cd-dvd/channel.nix"
    ];

    system.nixos.distroName = "Asterinas NixOS";
    system.nixos.label = "${version}";
    isoImage.appendToMenuLabel = " Installer";

    services.getty.autologinUser = pkgs.lib.mkForce "root";
    environment.systemPackages = [ installer ];
    environment.loginShellInit = ''
      if [ ! -e "$HOME/configuration.nix" ]; then
        # Copy the default configuration.nix to user's home directory.
        cp -L ${installer}/etc_nixos/configuration.nix $HOME
      fi

      ${pkgs.lib.optionalString autoInstall ''
        if [ "$(tty)" == "/dev/hvc0" ]; then
          echo "The installer automatically runs on /dev/hvc0!"
          install_aster_nixos.sh --config $HOME/configuration.nix --disk /dev/vda --force-format-disk || true
          poweroff
        fi
      ''}
    '';
  };
in (pkgs.nixos configuration).config.system.build.isoImage
