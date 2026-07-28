{ pkgs ? import <nixpkgs> { }, lib ? pkgs.lib, autoInstall ? false
, extra-substituters ? "", config-file-name ? "configuration.nix"
, extra-trusted-public-keys ? "", target_platform ? "x86_64-linux", version ? ""
, useAsterinasKernel ? false, ... }:
let
  installer = pkgs.callPackage ../aster_nixos_installer {
    inherit extra-substituters extra-trusted-public-keys config-file-name
      target_platform;
  };

  asterinasKernel = builtins.path {
    name = "aster-kernel-osdk-bin";
    path = ../../target/osdk/iso_root/boot/aster-kernel-osdk-bin;
  };

  aster-systemd = import ../etc_nixos/pkgs/systemd.nix { inherit pkgs; };

  baseConfiguration = {
    imports = [
      "${pkgs.path}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
      "${pkgs.path}/nixos/modules/installer/cd-dvd/channel.nix"
    ];

    system.nixos.distroName = "Asterinas NixOS";
    system.nixos.label = "${version}";
    isoImage.appendToMenuLabel =
      if useAsterinasKernel then " (Asterinas Kernel)" else " Installer";

    # DNS configuration (matching disk install in configuration.nix).
    environment.etc."resolv.conf".text = ''
      nameserver 8.8.8.8
    '';

    services.getty.autologinUser = pkgs.lib.mkForce "root";
    environment.systemPackages = [ installer ];

    environment.loginShellInit = ''
      if [ ! -e "$HOME/configuration.nix" ]; then
        cp -L ${installer}/etc_nixos/configuration.nix $HOME && chmod u+w $HOME/configuration.nix
      fi

      ${pkgs.lib.optionalString (autoInstall && !useAsterinasKernel) ''
        if [ "$(tty)" == "/dev/hvc0" ]; then
          echo "The installer automatically runs on /dev/hvc0!"
          aster-nixos-install --config $HOME/configuration.nix --disk /dev/vda --force-format-disk || true
          poweroff
        fi
      ''}
    '';
  };

  # The modified iso-image module replaces the standard NixOS iso-image.nix
  # with one adapted for the Asterinas kernel:
  #   - GRUB loads /boot/aster-kernel (not the Linux kernel)
  #   - The initrd is at /boot/initrd (the cpio with NixOS closure)
  #   - Kernel params do NOT include root= (ISO has no real root disk)
  #   - init= points to the NixOS toplevel init directly
  modifiedIsoImageModule = let
    originalNix = builtins.readFile
      "${pkgs.path}/nixos/modules/installer/cd-dvd/iso-image.nix";
    step1 = builtins.replaceStrings [ "cfg.system.build.initialRamdisk" ]
      [ "cfg.system.build.toplevel" ] originalNix;
    modifiedNix = builtins.replaceStrings [
      ''
        params = "init=''${cfg.system.build.toplevel}/init ''${toString cfg.boot.kernelParams} ''${toString params}";''
      ''
        image = "/boot/''${cfg.boot.kernelPackages.kernel + "/" + cfg.system.boot.loader.kernelFile}";''
      ''
        initrd = "/boot/''${cfg.system.build.toplevel + "/" + cfg.system.boot.loader.initrdFile}";''
      ''
        LINUX /boot/''${cfg.boot.kernelPackages.kernel + "/" + cfg.system.boot.loader.kernelFile}''
      "APPEND init=\${cfg.system.build.toplevel}/init \${toString cfg.boot.kernelParams} \${toString params}"
      ''
        INITRD /boot/''${cfg.system.build.toplevel + "/" + cfg.system.boot.loader.initrdFile}''
      "lib.optionalString cfg.isoImage.showConfiguration"
      "../../image/file-options.nix"
      "../../../lib/make-iso9660-image.nix"
    ] [
      ''
        params = "init=/init PATH=/bin:/nix/var/nix/profiles/system/sw/bin console=hvc0 ''${toString params}";''
      ''image = "/boot/aster-kernel";''
      ''initrd = "/boot/initrd";''
      "LINUX /boot/aster-kernel"
      "APPEND init=/init PATH=/bin:/nix/var/nix/profiles/system/sw/bin console=hvc0 \${toString params}"
      "INITRD /boot/initrd"
      "lib.optionalString true"
      "${pkgs.path}/nixos/modules/image/file-options.nix"
      "${pkgs.path}/nixos/lib/make-iso9660-image.nix"
    ] step1;
  in pkgs.writeText "iso-image-asterinas.nix" modifiedNix;

  # ISO-specific kernel configuration.
  # Imports asterinas-base.nix for the shared stage-1 init, initramfs, and
  # common settings.  Only overrides what differs from disk: GRUB config
  # (via modifiedIsoImageModule), fileSystems, and initrd content.
  asterinasKernelConfig = { config, ... }:
    let
      stage-1-init = import ../etc_nixos/modules/asterinas-stage-1-init.nix {
        inherit pkgs;
      };
    in {
      imports = [
        ../etc_nixos/modules/asterinas-base.nix
        ../etc_nixos/modules/systemd.nix
        "${modifiedIsoImageModule}"
      ];

      disabledModules =
        [ "${pkgs.path}/nixos/modules/installer/cd-dvd/iso-image.nix" ];

      isoImage.showConfiguration = false;

      isoImage.contents = lib.mkAfter [
        {
          source = asterinasKernel;
          target = "/boot/aster-kernel";
        }
        {
          # The ISO initrd must contain the full NixOS store closure (since there
          # is no real root disk), plus busybox and the shared stage-1 init from
          # asterinas-base.nix.  We reference the stage-1 init via the overlay
          # exported by asterinas-base.nix.
          source = let
            toplevel = config.system.build.toplevel;
            closureInfo = pkgs.closureInfo { rootPaths = [ toplevel ]; };
          in pkgs.runCommand "asterinas-initramfs" {
            nativeBuildInputs = [ pkgs.cpio pkgs.gzip ];
          } ''
            mkdir -p root/nix/store
            mkdir -p root/dev root/proc root/sys root/bin root/etc root/run root/var root/tmp root/usr/bin

            # Copy all files from the NixOS closure.
            while read -r path; do
              if [ -n "$path" ]; then
                cp -a "$path" root/nix/store/
              fi
            done < ${closureInfo}/store-paths

            # Register all store paths so Nix can find them.
            cp ${closureInfo}/registration root/nix/store/nix-path-registration

            # Copy busybox utilities to root/bin.
            cp -a ${pkgs.busybox}/bin/* root/bin/

            # Symlink sh, bash and env for script compatibility.
            ln -sfn ${pkgs.bash}/bin/sh root/bin/sh
            ln -sfn ${pkgs.bash}/bin/bash root/bin/bash
            ln -sfn ${pkgs.coreutils}/bin/env root/usr/bin/env

            # Symlink lib and lib64 to glibc for dynamic linker availability.
            ln -s ${pkgs.glibc}/lib root/lib
            ln -s ${pkgs.glibc}/lib root/lib64

            # Install the shared stage-1 init from asterinas-base.nix.
            cp ${stage-1-init} root/init
            chmod +x root/init

            # Pack into cpio.gz format.
            cd root
            find . -mindepth 1 | cpio -o -H newc | gzip -9 > $out
          '';
          target = "/boot/initrd";
        }
      ];

      # The ISO rootfs is the initrd itself (ramfs).  We only need noauto
      # stubs for paths the NixOS installer might reference.
      # We must NOT define /nix/store (or .ro-store / .rw-store) here because
      # systemd would otherwise mount an empty ramfs over our populated Nix store.
      fileSystems = lib.mkOverride 0 {
        "/iso" = {
          device = "none";
          fsType = "ramfs";
          options = [ "noauto" ];
        };
      };

      # Prevent NixOS from building a Linux initrd — we provide our own
      # (busybox stage-1 + NixOS store closure via asterinas-base.nix).
      boot.initrd.enable = lib.mkForce false;
      boot.initrd.availableKernelModules = lib.mkForce [ ];
      boot.initrd.kernelModules = lib.mkForce [ ];
    };

  configuration = {
    imports = [ baseConfiguration ]
      ++ pkgs.lib.optionals useAsterinasKernel [ asterinasKernelConfig ];

    # Force aster-systemd for the systemd package when using Asterinas kernel.
    # The stock systemdMinimal lacks files referenced by NixOS modules
    # (systemd-logind, systemd-user-sessions, systemd-bsod, etc.).
    systemd.package = pkgs.lib.mkForce
      (if useAsterinasKernel then aster-systemd else pkgs.systemd);

    nixpkgs.overlays = pkgs.lib.optionals useAsterinasKernel [
      (import ../etc_nixos/overlays/systemd/default.nix)
      (import ../etc_nixos/overlays/switch-to-configuration-ng/default.nix)
      (import ../etc_nixos/overlays/hello-asterinas/default.nix)
    ];
  };
in (pkgs.nixos configuration).config.system.build.isoImage
