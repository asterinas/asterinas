# The system-wide settings for AsterNixOS.

{ config, lib, pkgs, ... }: {
  # Imports all Nix files located under the 'modules' directory.
  #
  # Each Nix file within the 'modules' directory contributes a specific part to the overall 'configuration.nix'.
  # For instance, 'core.nix' typically handles configurations
  # related to the system's core functionalities and boot process.
  #
  # To maintain modularity,
  # distinct aspects of the system's configuration
  # should be organized into separate Nix files under the 'modules' directory and subsequently imported here.
  #
  # If a module's content is optional,
  # its activation should be controlled by an enable/disable switch defined in the top-level 'configuration.nix'.
  #
  # The content defined in these module files must adhere to the options permissible within 'configuration.nix'.
  # For a comprehensive list of available options,
  # please refer to https://search.nixos.org/options.
  imports = [
    ./modules/core.nix
    ./modules/xfce/default.nix
    ./modules/container.nix
    ./modules/systemd.nix
  ];

  # Overlays provide patches to 'nixpkgs' that enable these packages to run effectively on AsterNixOS.
  # For details on the overlay file definition format,
  # please refer to https://nixos.org/manual/nixpkgs/stable/#sec-overlays-definition.
  config.nixpkgs.overlays = [
    (import ./overlays/desktop/default.nix)
    (import ./overlays/hello-asterinas/default.nix)
    (import ./overlays/podman/default.nix)
    (import ./overlays/switch-to-configuration-ng/default.nix)
    (import ./overlays/systemd/default.nix)
  ];

  # The Asterinas NixOS special options.
  options = {
    aster_nixos = {
      kernel = lib.mkOption {
        type = lib.types.path;
        default = "@aster-kernel@";
        description = "The path to the kernel image.";
      };
      disable-systemd = lib.mkOption {
        type = lib.types.enum [ "true" "false" ];
        default = "@aster-disable-systemd@";
        description = "Whether to disable systemd in stage 2 init.";
      };
      stage-2-hook = lib.mkOption {
        type = lib.types.str;
        default = "@aster-stage-2-hook@";
        description = "Stage 2 init command (fallback when systemd disabled).";
      };
      log-level = lib.mkOption {
        type = lib.types.enum [ "error" "warn" "info" "debug" "trace" ];
        default = "@aster-log-level@";
        description = "The log level of Asterinas NixOS.";
      };
      console = lib.mkOption {
        type = lib.types.enum [ "tty0" "hvc0" ];
        default = "@aster-console@";
        description = "The console device.";
      };
      break-into-stage-1-shell = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description =
          "If set to true, the system will not proceed to switch to the root filesystem after initial boot. Instead, it will drop into an initramfs shell. This is primarily intended for debugging purposes.";
      };
      substituters = lib.mkOption {
        type = lib.types.str;
        default = "@aster-substituters@";
        description = "The substituters fo Asterinas NixOS.";
      };
      trusted-public-keys = lib.mkOption {
        type = lib.types.str;
        default = "@aster-trusted-public-keys@";
        description = "The trusted public keys of Asterinas NixOS.";
      };
    };
  };
}
