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
    (import ./overlays/hello-asterinas/default.nix)
    (import ./overlays/desktop/default.nix)
    (import ./overlays/podman/default.nix)
    (import ./overlays/systemd/default.nix)
  ];

  # The Asterinas NixOS special options.
  options = {
    asterinas = {
      kernel = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = lib.maybeEnv "NIXOS_KERNEL" null;
        description = "The path to the kernel image.";
      };

      stage-1-init = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = lib.maybeEnv "NIXOS_STAGE_1_INIT" null;
        description = "The path to the stage 1 init script.";
      };

      resolv-conf = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = lib.maybeEnv "NIXOS_RESOLV_CONF" null;
        description = "The path to the resolv.conf file.";
      };

      disable-systemd = lib.mkOption {
        type = lib.types.enum [ "true" "false" ];
        default = lib.maybeEnv "NIXOS_DISABLE_SYSTEMD" "false";
        description = "Whether to disable systemd in stage 2 init.";
      };

      stage-2-hook = lib.mkOption {
        type = lib.types.str;
        default = lib.maybeEnv "NIXOS_STAGE_2_INIT" "/bin/sh";
        description = "Stage 2 init command (fallback when systemd disabled).";
      };

      log-level = lib.mkOption {
        type = lib.types.enum [ "error" "warn" "info" "debug" "trace" ];
        default = lib.maybeEnv "LOG_LEVEL" "error";
        description = "The log level of Asterinas NixOS.";
      };

      console = lib.mkOption {
        type = lib.types.enum [ "tty0" "hvc0" ];
        default = lib.maybeEnv "CONSOLE" "hvc0";
        description = "The console device.";
      };

      break-into-stage-1-shell = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description =
          "If set to true, the system will not proceed to switch to the root filesystem after initial boot. Instead, it will drop into an initramfs shell. This is primarily intended for debugging purposes.";
      };

      run-test = lib.mkOption {
        type = lib.types.bool;
        default = lib.maybeEnv "NIXOS_RUN_TEST" "false" == "true";
        description = "Whether to automatically run tests after boot.";
      };

      test-command = lib.mkOption {
        type = lib.types.str;
        default = lib.maybeEnv "NIXOS_TEST_COMMAND" "hello-asterinas";
        description = "The command(s) to execute when run-test is enabled.";
      };
    };
  };
}
