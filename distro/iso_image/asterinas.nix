# SPDX-License-Identifier: MPL-2.0
{
  pkgs ? import ../nixpkgs.nix { },
  target_platform ? "x86_64-linux",
  version ? "",
  substituters ? "",
  trusted-public-keys ? "",
  logLevel ? "error",
  config-file-name ? "configuration.nix",
}:
let
  inherit (pkgs) lib;
  nixpkgsPath = pkgs.path;
  busybox = pkgs.busybox.override { enableStatic = true; };
  kernel = builtins.path {
    name = "asterinas-osdk-bin";
    path = ../../target/osdk/iso_root/boot/asterinas-osdk-bin;
  };
  system =
    (pkgs.nixos {
      imports = [
        "${nixpkgsPath}/nixos/modules/profiles/minimal.nix"
        (./. + "/${config-file-name}")
      ];
      nixpkgs.hostPlatform = target_platform;
      nix.settings.substituters = lib.splitString " " substituters;
      nix.settings.trusted-public-keys = lib.splitString " " trusted-public-keys;
    }).config.system.build.toplevel;
  stage-1-init = import ./stage-1-init.nix { inherit pkgs system busybox; };
  closure = pkgs.closureInfo {
    rootPaths = [ system ];
  };
  initramfs =
    pkgs.runCommand "asterinas-iso-initramfs"
      {
        nativeBuildInputs = [
          pkgs.cpio
          pkgs.gzip
        ];
      }
      ''
        mkdir -p root/iso-root/nix/store root/iso-root/{dev,proc,sys,run,tmp,etc,var,bin}
        while read -r path; do
          cp -a "$path" root/iso-root/nix/store/
        done < ${closure}/store-paths
        cp ${closure}/registration root/iso-root/nix/store/nix-path-registration

        # Stage 1 needs only BusyBox outside the live root.
        mkdir -p root/nix/store
        cp -a ${busybox} root/nix/store/
        cp ${stage-1-init} root/init
        chmod +x root/init
        cd root
        find . -exec touch -h -d '@1' '{}' +
        find . -print0 | sort -z | cpio --quiet --null -o -H newc -R +0:+0 --reproducible | gzip -n > $out
      '';
  grub = pkgs.callPackage ../../tools/dev_env/nix/packages/grub.nix {
    grub2 = pkgs.pkgsCross.gnu64.grub2.override { efiSupport = true; };
    grub2-host = pkgs.grub2.override { efiSupport = true; };
  };
in
assert lib.assertMsg (
  target_platform == "x86_64-linux"
) "Asterinas ISO boot currently supports only x86_64-linux";
pkgs.runCommand "asterinas-live-${version}.iso"
  {
    nativeBuildInputs = [
      grub
      pkgs.xorriso
      pkgs.mtools
    ];
  }
  ''
    mkdir -p image/boot/grub $out/iso
    cp ${kernel} image/boot/kernel
    cp ${initramfs} image/boot/initrd
    cat > image/boot/grub/grub.cfg <<'EOF'
    set timeout=0
    menuentry "Asterinas NixOS Live" {
      linux /boot/kernel earlycon console=hvc0 loglevel=${logLevel} rdinit=/init
      initrd /boot/initrd
    }
    EOF
    grub-mkrescue -o "$out/iso/asterinas-live.iso" image
  ''
