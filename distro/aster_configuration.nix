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
  nixpkgs.overlays = [
    (import ./overlays/hello-asterinas/default.nix)
    (import ./overlays/desktop/default.nix)
    (import ./overlays/podman/default.nix)
    (import ./overlays/systemd/default.nix)
    (import ./overlays/switch-to-configuration-ng/default.nix)
  ];
}
