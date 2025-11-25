{ config, lib, pkgs, ... }:
let
  kernel = builtins.path { path = builtins.getEnv "NIXOS_KERNEL"; };
  stage-1-init = builtins.path { path = builtins.getEnv "NIXOS_STAGE_1_INIT"; };
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
  resolv-conf = builtins.path { path = builtins.getEnv "NIXOS_RESOLV_CONF"; };
  # If set to "1", the system will not proceed to switch to the root filesystem after
  # initial boot. Instead, it will drop into an initramfs shell. This is primarily
  # intended for debugging purposes.
  break-into-stage1-shell = "0";
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
    rm -rf /etc/resolv.conf
    ln -s ${resolv-conf} /etc/resolv.conf
    PATH=$PATH:/nix/var/nix/profiles/system/sw/bin:~/.nix-profile/bin
    if [ "${builtins.getEnv "NIXOS_DISABLE_SYSTEMD"}" = "true" ]; then
      ${builtins.getEnv "NIXOS_STAGE_2_INIT"}
    fi
  '';
  system.systemBuilderCommands = ''
    echo "PATH=/bin:/nix/var/nix/profiles/system/sw/bin ostd.log_level=${
      builtins.getEnv "LOG_LEVEL"
    } console=${
      builtins.getEnv "CONSOLE"
    } -- sh /init root=/dev/vda2 init=/nix/var/nix/profiles/system/stage-2-init rd.break=${break-into-stage1-shell}"  > $out/kernel-params
    mv $out/init $out/stage-2-init
    sed -i 's_^\([[:space:]]*\)\(exec > >(tee -i /run/log/stage-2-init.log) 2>&1\)$_\1# \2_' $out/stage-2-init
    if [ "${builtins.getEnv "NIXOS_DISABLE_SYSTEMD"}" = "true" ]; then
      sed -i 's/^[[:space:]]*echo "starting systemd..."$/# &/' $out/stage-2-init
      sed -i 's/^[[:space:]]*exec \/run\/current-system\/systemd\/lib\/systemd\/systemd "$@"$/# &/' $out/stage-2-init
    fi
    rm -rf $out/init
    ln -s /bin/busybox $out/init
    ln -s ${kernel} $out/kernel
    ln -s ${initramfs}/initrd $out/initrd
  '';
  system.activationScripts.modprobe = lib.mkForce "";

  nix.settings = {
    filter-syscalls = false;
    require-sigs = false;
    sandbox = false;
  };
}
