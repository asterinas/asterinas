{ config, lib, pkgs, ... }:
let
  kernel = builtins.path { path = config.asterinas.kernel; };
  stage-1-init = builtins.path { path = config.asterinas.stage-1-init; };
  initramfs = pkgs.makeInitrd {
    contents = [
      {
        object = "${pkgs.busybox}/bin";
        symlink = "/bin";
      }
      {
        object = "${stage-1-init}";
        symlink = "/init";
      }
    ];
  };
  resolv-conf = builtins.path { path = config.asterinas.resolv-conf; };
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
    ${if config.asterinas.resolv-conf != null then ''
      rm -rf /etc/resolv.conf
      ln -s ${resolv-conf} /etc/resolv.conf
    '' else
      lib.concatStrings (lib.forEach config.networking.nameservers (ns: ''
        echo 'nameserver ${ns}' >> /etc/resolv.conf
      ''))}

    if [ "${config.asterinas.disable-systemd}" = "true" ]; then
      ${config.asterinas.stage-2-hook}
    fi
    ${lib.optionalString config.asterinas.run-test ''
      cp /etc/profile /etc/profile.bak
      echo '
      # --- Run tests in the hvc0 terminal ---
      if [ "$(tty)" = "/dev/hvc0" ]; then
        # Resume /etc/profile
        cp /etc/profile.bak /etc/profile
        rm /etc/profile.bak
        # Execute specified tests
        ${config.asterinas.test-command}
        # Wait and poweroff 
        sleep 2
        poweroff
      fi
      ' >> /etc/profile
    ''}
  '';
  system.systemBuilderCommands = ''
    echo "PATH=/bin:/nix/var/nix/profiles/system/sw/bin ostd.log_level=${config.asterinas.log-level} console=${config.asterinas.console} -- sh /init root=/dev/vda2 init=/nix/var/nix/profiles/system/stage-2-init rd.break=${
      if config.asterinas.break-into-stage-1-shell then "1" else "0"
    }"  > $out/kernel-params
    mv $out/init $out/stage-2-init
    sed -i 's_^\([[:space:]]*\)\(exec > >(tee -i /run/log/stage-2-init.log) 2>&1\)$_\1# \2_' $out/stage-2-init
    if [ "${config.asterinas.disable-systemd}" = "true" ]; then
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
