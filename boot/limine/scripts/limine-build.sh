#! /bin/sh

set -xe

LIMINE_GIT_URL="https://github.com/limine-bootloader/limine.git"

# Cargo passes the path to the built executable as the first argument.
KERNEL=$1

# Clone the `limine` repository if we don't have it yet.
if [ ! -d target/limine ]; then
    git clone $LIMINE_GIT_URL --depth=1 --branch v4.x-branch-binary target/limine
fi

cd target/limine
make
cd -

# Copy the needed files into an ISO image.
mkdir -p target/iso_root
cp $KERNEL target/iso_root/jinux
cp boot/limine/conf/limine.cfg target/iso_root
cp target/limine/limine.sys target/iso_root
cp target/limine/limine-cd.bin target/iso_root
cp target/limine/limine-cd-efi.bin target/iso_root

# Copy ramdisk
cp regression/build/ramdisk.cpio.gz target/iso_root

xorriso -as mkisofs                                             \
    -b limine-cd.bin                                            \
    -no-emul-boot -boot-load-size 4 -boot-info-table            \
    --efi-boot limine-cd-efi.bin                                \
    -efi-boot-part --efi-boot-image --protective-msdos-label    \
    target/iso_root -o $KERNEL.iso

# For the image to be bootable on BIOS systems, we must run `limine-deploy` on it.
target/limine/limine-deploy $KERNEL.iso
