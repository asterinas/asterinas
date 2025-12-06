{ distro, kernel, tools, autoInstall ? false, ... }:
let
  # Pinned nixpkgs (channel: nixos-25.05, release date: 2025-07-01)
  pkgs = import (fetchTarball
    "https://github.com/NixOS/nixpkgs/archive/c0bebd16e69e631ac6e52d6eb439daba28ac50cd.tar.gz") {
      config = { allowUnfree = true; };
    };

  distroPath = builtins.path { path = distro; };
  kernelPath = builtins.path { path = kernel; };
  toolsPath = builtins.path { path = tools; };
  installer = pkgs.replaceVarsWith {
    src = "${toolsPath}/iso_installer.sh";
    isExecutable = true;
    replacements = {
      shell = "${pkgs.bash}/bin/sh";
      inherit distroPath kernelPath toolsPath autoInstall;
    };
  };
  configuration = {
    imports = [
      "${pkgs.path}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
      "${pkgs.path}/nixos/modules/installer/cd-dvd/channel.nix"
    ];

    services.getty.autologinUser = pkgs.lib.mkForce "root";
    environment.loginShellInit = "${installer}";
    nix.settings = {
      substituters = [ "https://test-21.cachix.org" ];
      trusted-public-keys =
        [ "test-21.cachix.org-1:RpzafHw7UMo9MI1R0CKxeGq9zuc23NbgVBg7QzY5u60=" ];
    };
  };
in (pkgs.nixos configuration).config.system.build.isoImage
