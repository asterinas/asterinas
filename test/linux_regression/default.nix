# SPDX-License-Identifier: MPL-2.0
{
  system ? builtins.currentSystem,
}:
if system != "x86_64-linux" then
  throw "Linux regression runner only supports x86_64-linux (got ${system})"
else
  let
    pkgs = import ../../distro/nixpkgs.nix { inherit system; };
    kernel = pkgs.fetchurl {
      url = "https://raw.githubusercontent.com/asterinas/linux_binary_cache/8e6e8ad382aac7e605d7a058811d09bce478cd7d/6.16.0/vmlinuz";
      hash = "sha256-HT6KiGY7ygemKqdpVZ9JFT3b3aEGKwziwdydd7BtJOc=";
    };

    # Build the suite for the VM's `/dev/vd*` disks rather than loop devices.
    regression = pkgs.callPackage ../initramfs/nix/regression {
      testPlatform = "linux_vm";
    };

    # Dereference category symlinks so `apply_blocklist.sh` can replace tests in place.
    filteredTests = pkgs.runCommand "linux-regression-tests" { } ''
      cp -rL ${regression.package} "$out"
      chmod -R u+w "$out"
      bash ${./apply_blocklist.sh} ${./blocklist} "$out"
    '';
    guestClosure = pkgs.writeClosure [
      filteredTests
      pkgs.busybox
    ];

    # Build a minimal bootable ext4 root disk image for the Linux VM.
    rootfs = pkgs.runCommand "linux-regression-root.ext4" { nativeBuildInputs = [ pkgs.e2fsprogs ]; } ''
      set -o pipefail
      # Omit `/exfat` so tests requiring exFAT cannot silently run on ext4.
      mkdir -p root/{dev,etc,ext2,proc,sys,tmp,usr,nix/store}
      cd root

      install -m 0644 ${../initramfs/etc/passwd} etc/passwd
      install -m 0644 ${../initramfs/etc/group} etc/group
      install -m 0644 ${../initramfs/etc/hosts} etc/hosts
      install -m 0755 ${./init.sh} init
      install -m 0755 ${./test_session.sh} test_session.sh
      cp -r ${filteredTests} test
      ln -s ${pkgs.busybox}/bin bin
      ln -s ../bin usr/bin

      while IFS= read -r store_path; do
        if [[ "$store_path" != "${filteredTests}" ]]; then
          cp -a "$store_path" nix/store/
        fi
      done < ${guestClosure}

      tree_mib=$(du --apparent-size -sm . | cut -f1)
      headroom_mib=256
      truncate -s "$((tree_mib + headroom_mib))M" "$out"
      tar --owner=0 --group=0 -cf - . | mkfs.ext4 -q -d - "$out"
    '';

    # Bundle the host tools that create data disk images and run QEMU.
    tools = pkgs.buildEnv {
      name = "linux-regression-tools";
      paths = with pkgs; [
        coreutils
        e2fsprogs
        exfatprogs
        gnumake
        qemu_test
      ];
      pathsToLink = [ "/bin" ];
    };
  in
  pkgs.linkFarm "linux-regression-vm" {
    inherit kernel rootfs tools;
  }
