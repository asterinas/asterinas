set -ex

# args: start_linux.sh [core_count]

CORE_COUNT=${1:-128}

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
INITRAMFS_DIR="$SCRIPT_DIR/../test/build/initramfs.cpio.gz"
EXT2_IMG_DIR="$SCRIPT_DIR/../test/build/ext2.img"
LINUX_KERNEL="/root/linux-6.13.8/bzImage"

# Disable unsupported ext2 features of Asterinas on Linux to ensure fairness
mke2fs -F -O ^ext_attr -O ^resize_inode -O ^dir_index ${EXT2_IMG_DIR}

pushd "$SCRIPT_DIR/.."
make initramfs
popd

/usr/local/qemu/bin/qemu-system-x86_64 \
    --no-reboot \
    -smp $CORE_COUNT \
    -m 256G \
    -machine q35,kernel-irqchip=split \
    -cpu host,migratable=off,-pcid,+x2apic \
    --enable-kvm \
    -kernel ${LINUX_KERNEL} \
    -initrd ${INITRAMFS_DIR} \
    -drive if=none,format=raw,id=x0,file=${EXT2_IMG_DIR} \
    -device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,config-wce=off,request-merging=off,write-cache=off,backend_defaults=off,discard=off,event_idx=off,indirect_desc=off,ioeventfd=off,queue_reset=off \
    -drive if=none,format=raw,id=x2,file=./test/build/bench_data.img \
    -device virtio-blk-pci,bus=pcie.0,addr=0x7,drive=x2,serial=vbench,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,config-wce=off,request-merging=off,write-cache=off,backend_defaults=off,discard=off,event_idx=off,indirect_desc=off,ioeventfd=off,queue_reset=off \
    -append 'console=ttyS0 rdinit=/usr/bin/busybox quiet mitigations=off hugepages=0 transparent_hugepage=never SHELL=/bin/sh LOGNAME=root HOME=/ USER=root PATH=/bin:/benchmark -- sh -l' \
    -qmp tcp:127.0.0.1:${QMP_PORT-9889},server,nowait \
    -nographic