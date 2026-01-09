{ config, lib, pkgs, options, ... }:
let
  kernel = builtins.path {
    name = "aster-kernel-osdk-bin";
    path = config.aster_nixos.kernel;
  };
  stage-1-init = pkgs.writeShellScript "stage-1-init" ''
    #!/bin/sh
    # SPDX-License-Identifier: MPL-2.0

    NEW_ROOT=""
    NEW_INIT=""
    BREAK=""
    ARGS=""

    for arg in "$@"; do
      case "$arg" in
        root=*)
          NEW_ROOT=''${arg#root=}
          ;;
        init=*)
          NEW_INIT=''${arg#init=}
          ;;
        rd.break=*)
          BREAK=''${arg#rd.break=}
          ;;
        *)
          ARGS="$ARGS $arg"
          ;;
      esac
    done

    if [ "$BREAK" = "1" ]; then
      echo "Breaking into initramfs shell..."
      exec /bin/sh
    fi

    if [ -z "$NEW_ROOT" ] || [ -z "$NEW_INIT" ]; then
      echo "Error: 'root=' and 'init=' parameters are required."
      exit 1
    fi

    mkdir /sysroot
    mount -t ext2 "$NEW_ROOT" /sysroot
    mount -t proc none /sysroot/proc
    mount --move /dev /sysroot/dev

    exec switch_root /sysroot "$NEW_INIT" "$ARGS"
  '';

  initramfs = pkgs.makeInitrd {
    contents = [
      {
        object = "${pkgs.busybox}/bin";
        symlink = "/bin";
      }
      {
        object = stage-1-init;
        symlink = "/init";
      }
    ];
  };
in {
  boot.loader.grub.enable = true;
  boot.loader.grub.efiSupport = true;
  boot.loader.grub.device = "nodev";
  boot.loader.grub.efiInstallAsRemovable = true;
  boot.initrd.enable = false;
  boot.kernel.enable = false;
  # Hook function will be called in stage-2-init and before running systemd.
  boot.postBootCommands = ''
    echo "Executing postBootCommands..."
    if [ "${config.aster_nixos.disable-systemd}" = "true" ]; then
      ${config.aster_nixos.stage-2-hook}
    fi
  '';
  # Suppress error and warning messages of systemd.
  # TODO: Fix errors and warnings from systemd and remove this setting.
  environment.sessionVariables = { SYSTEMD_LOG_LEVEL = "crit"; };
  system.systemBuilderCommands = ''
    echo "PATH=/bin:/nix/var/nix/profiles/system/sw/bin ostd.log_level=${config.aster_nixos.log-level} console=${config.aster_nixos.console} -- sh /init root=/dev/vda2 init=/nix/var/nix/profiles/system/stage-2-init rd.break=${
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
    ln -s ${initramfs}/initrd $out/initrd
  '';
  system.activationScripts.modprobe = lib.mkForce "";

  nix.nixPath = options.nix.nixPath.default
    ++ [ "nixpkgs-overlays=/etc/nixos/overlays" ];
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

  # FIXME: Currently, during `nixos-rebuild`, `texinfo/install-info` encounters a `SIGBUS`.
  documentation.info.enable = false;
}
