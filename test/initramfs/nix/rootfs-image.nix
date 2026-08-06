{ stdenvNoCC, pkgsBuildBuild, initramfs, }:
stdenvNoCC.mkDerivation {
  name = "rootfs-image";
  nativeBuildInputs = with pkgsBuildBuild; [ e2fsprogs ];
  buildCommand = ''
    truncate -s 256M "$out"
    mkfs.ext2 -b 4096 -d ${initramfs} -F "$out"
  '';
}
