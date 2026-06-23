{ stdenvNoCC, pkgsBuildBuild, initramfs, }:
stdenvNoCC.mkDerivation {
  name = "rootfs-image";
  nativeBuildInputs = with pkgsBuildBuild; [ e2fsprogs ];
  buildCommand = ''
    rootfs_dir=$(mktemp -d)
    cp -r ${initramfs}/* "$rootfs_dir"/

    truncate -s 256M "$out"
    mkfs.ext2 -b 4096 -d "$rootfs_dir" -F "$out"
  '';
}
