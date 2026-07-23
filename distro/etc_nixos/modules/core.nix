{ config, lib, pkgs, options, ... }:
let
  kernel = builtins.path {
    name = "aster-kernel-osdk-bin";
    path = config.aster_nixos.kernel;
  };
in {
  imports = [ ./asterinas-base.nix ];

  boot.loader.grub.enable = true;
  boot.loader.grub.efiSupport = true;
  boot.loader.grub.device = "nodev";
  boot.loader.grub.efiInstallAsRemovable = true;
  # Hook function will be called in stage-2-init and before running systemd.
  boot.postBootCommands = ''
    echo "Executing postBootCommands..."
    if [ "${config.aster_nixos.disable-systemd}" = "true" ]; then
      ${config.aster_nixos.stage-2-hook}
    fi
  '';

  systemd.services.restore-devices = {
    description = "Restore kernel-created block devices";
    wantedBy = [ "sysinit.target" ];
    before = [ "local-fs-pre.target" "systemd-tmpfiles-setup-dev.service" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart =
        "${pkgs.bash}/bin/bash -c 'if [ -d /run/initramfs/dev ]; then cp -a /run/initramfs/dev/vd* /dev/ 2>/dev/null || true; cp -a /run/initramfs/dev/nvme* /dev/ 2>/dev/null || true; fi'";
    };
  };
  system.systemBuilderCommands = ''
    echo "PATH=/bin:/nix/var/nix/profiles/system/sw/bin earlycon loglevel=${config.aster_nixos.log-level} console=${config.aster_nixos.console} -- sh /init root=/dev/vda2 init=/nix/var/nix/profiles/system/stage-2-init rd.break=${
      if config.aster_nixos.break-into-stage-1-shell then "1" else "0"
    }"  > $out/kernel-params
    mv $out/init $out/stage-2-init
    sed -i 's_^\([[:space:]]*\)\(exec > >(tee -i /run/log/stage-2-init.log) 2>&1\)$_\1# \2_' $out/stage-2-init
    if [ "${config.aster_nixos.disable-systemd}" = "true" ]; then
      sed -i 's/^[[:space:]]*echo "starting systemd..."$/# &/' $out/stage-2-init
      sed -i 's/^[[:space:]]*exec \/run\/current-system\/systemd\/lib\/systemd\/systemd "$@"$/# &/' $out/stage-2-init
    fi
    rm -rf $out/init
    ln -s /bin/busybox $out/init
    ln -s ${kernel} $out/kernel
    ln -s ${pkgs.asterinas-initramfs}/initrd $out/initrd
  '';

  nix.nixPath = options.nix.nixPath.default
    ++ [ "nixpkgs-overlays=/etc/nixos/overlays" ];
  nix.channel.enable = false;
  nix.settings = {
    filter-syscalls = false;
    require-sigs = false;
    sandbox = false;
    # FIXME: Support Nix build users (nixbld*) and remove this setting. For detailed gaps, see
    # <https://github.com/asterinas/asterinas/issues/2672>.
    build-users-group = "";
    substituters = [ "${config.aster_nixos.substituters}" ];
    trusted-public-keys = [ "${config.aster_nixos.trusted-public-keys}" ];
  };
}
