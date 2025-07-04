{ stdenv, pkgsBuildBuild, initramfs, compressed, }:
stdenv.mkDerivation {
  name = "initramfs-image";
  nativeBuildInputs = with pkgsBuildBuild; [ cpio gzip ];
  buildCommand = ''
    pushd $(mktemp -d)
    cp -r ${initramfs}/* ./
    chmod -R 0755 benchmark
    chmod -R 0755 etc
    chmod -R 0755 opt
    chmod -R 0755 test
    chmod -R 0755 ext2
    chmod -R 0755 exfat
    chmod -R 0755 var
    chmod -R 1777 tmp

    if [ "${toString compressed}" == "1" ]; then
      find . -print0 | cpio -o -H newc --null | gzip > $out
    else
      find . -print0 | cpio -o -H newc --null | cat > $out
    fi
    popd
  '';
}
