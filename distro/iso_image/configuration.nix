# SPDX-License-Identifier: MPL-2.0
# The default Live system; test suites extend this module with their extra config.
{ lib, pkgs, ... }:
{
  imports = [ ../etc_nixos/modules/systemd.nix ];
  nixpkgs.overlays = [
    (import ../etc_nixos/overlays/systemd/default.nix)
    (import ../etc_nixos/overlays/hello-asterinas/default.nix)
  ];
  system.stateVersion = "26.05";
  system.nixos.distroName = "Asterinas NixOS";
  networking.hostName = "asterinas";
  users.users.root.initialHashedPassword = "";
  boot.loader.grub.enable = false;
  # Stage 1 owns the live mounts. An fstab entry would make systemd try
  # to recreate the overlay without access to its lower and upper paths.

  networking.resolvconf.enable = false;
  environment.etc."resolv.conf".text = "nameserver 8.8.8.8\n";
  environment.systemPackages = with pkgs; [
    hello-asterinas
    gitMinimal
  ];
  system.tools.nixos-install.enable = false;
  system.tools.nixos-generate-config.enable = false;
  system.tools.nixos-enter.enable = false;
  system.tools.nixos-build-vms.enable = false;
  boot.kernel.enable = false;
  boot.initrd.enable = false;
  system.activationScripts.modprobe = lib.mkForce "";
  environment.sessionVariables.SYSTEMD_LOG_LEVEL = "crit";
  documentation.info.enable = false;
  boot.postBootCommands = ''
    ln -sfn sh /bin/bash
  '';
  nix.nixPath = [ "nixpkgs=${pkgs.path}" ];
  system.extraDependencies = [ (builtins.storePath pkgs.path) ];
  nix.settings = {
    experimental-features = [
      "nix-command"
      "flakes"
    ];
    filter-syscalls = false;
    sandbox = false;
    build-users-group = "";
  };
  systemd.services.register-nix-paths = {
    description = "Register the live Nix store";
    unitConfig.DefaultDependencies = false;
    wantedBy = [ "sysinit.target" ];
    before = [
      "sysinit.target"
      "nix-daemon.service"
      "nix-daemon.socket"
    ];
    after = [ "local-fs.target" ];
    serviceConfig.Type = "oneshot";
    script = ''
      ${pkgs.nix}/bin/nix-store --load-db < /nix/store/nix-path-registration
      ${pkgs.nix}/bin/nix-env -p /nix/var/nix/profiles/system --set /run/current-system
    '';
  };
}
