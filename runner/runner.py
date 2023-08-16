import subprocess
import argparse
import shutil
import os
import random

arg_parser = argparse.ArgumentParser()
arg_parser.add_argument('path', type=str, help='The Jinux binary path.')
arg_parser.add_argument('kcmdline', type=str, help='Provide the kernel commandline, which specifies the init process.')
arg_parser.add_argument('--enable-kvm', action='store_true', default=False, help='Enable KVM when running QEMU.')
arg_parser.add_argument('--emulate-iommu', action='store_true', default=False, help='Emulate Intel IOMMU by QEMU.')
arg_parser.add_argument('--do-kmode-test', action='store_true', default=False, help='Do kernel mode testing.')
args = arg_parser.parse_args()

def random_hostfwd_port():
    with open('/proc/sys/net/ipv4/ip_local_port_range', 'r') as f:
        start_port, end_port = map(int, f.readline().split())
    l = end_port - start_port
    port1 = start_port + random.randint(1, l // 2)
    port2 = port1 + random.randint(1, l // 2)
    return f",hostfwd=tcp::{port1}-:22,hostfwd=tcp::{port2}-:8080"

def virtio_device_arg(device_str):
    return device_str + (",iommu_platform=on,ats=on" if args.emulate_iommu else "")

COMMON_ARGS = [
    "--no-reboot",
    "-machine", "q35,kernel-irqchip=split",
    "-cpu", "Icelake-Server,+x2apic",
    "-m", "2G",
    "-nographic", # TODO: figure out why grub can't shown up without it
    "-monitor", "vc",
    "-serial", "mon:stdio",
    "-display", "none",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-netdev", "user,id=net01" + random_hostfwd_port(),
    "-object", "filter-dump,id=filter0,netdev=net01,file=virtio-net.pcap",
    "-device", virtio_device_arg("virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off"),
    "-device", virtio_device_arg("virtio-keyboard-pci,disable-legacy=on,disable-modern=off"),
    "-device", virtio_device_arg("virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off"),
]

IOMMU_DEVICE_ARGS = [
    "-device", "intel-iommu,intremap=on,device-iotlb=on",
    "-device", "ioh3420,id=pcie.0,chassis=1",
]

def main():
    qemu_args = COMMON_ARGS[:]
    if args.enable_kvm:
        qemu_args += ['-enable-kvm']

    if args.emulate_iommu:
        qemu_args += IOMMU_DEVICE_ARGS

    qemu_args += ["-drive", create_fs_image()]
    qemu_args += ["-cdrom", create_bootdev_image()]

    qemu_cmd = ['qemu-system-x86_64'] + qemu_args

    print(f'running {qemu_cmd}')

    exit_status = subprocess.run(qemu_cmd).returncode
    if exit_status != 0:
        # FIXME: Exit code manipulation is not needed when using non-x86 QEMU
        qemu_exit_code = exit_status
        kernel_exit_code = qemu_exit_code >> 1
        if kernel_exit_code == 0x10:  # jinux_frame::QemuExitCode::Success
            os._exit(0)
        elif kernel_exit_code == 0x20:  # jinux_frame::QemuExitCode::Failed
            os._exit(1)
        else:
            os._exit(qemu_exit_code)

def generate_grub_cfg(template_filename, target_filename):
    with open(template_filename, 'r') as file:
        buffer = file.read()

    replaced = buffer.replace('#KERNEL_COMMAND_LINE#', args.kcmdline)\
                     .replace('#GRUB_TIMEOUT_STYLE#', 'hidden' if args.do_kmode_test else 'menu')\
                     .replace('#GRUB_TIMEOUT#', '0' if args.do_kmode_test else '1')

    with open(target_filename, 'w') as file:
        file.write(replaced)

def create_bootdev_image():
    dir_path = os.path.dirname(args.path)
    name = os.path.basename(args.path)
    iso_path = os.path.join(dir_path, name + '.iso')

    if os.path.exists('target/iso_root'):
        shutil.rmtree('target/iso_root')

    os.makedirs('target/iso_root/boot/grub')

    shutil.copy2(args.path, 'target/iso_root/boot/jinux')
    generate_grub_cfg('runner/grub/grub.cfg.template', 'target/iso_root/boot/grub/grub.cfg')
    shutil.copy2('regression/build/initramfs.cpio.gz', 'target/iso_root/boot/initramfs.cpio.gz')

    status = subprocess.run(['grub-mkrescue', '-o', iso_path, 'target/iso_root']).returncode

    if status != 0:
        raise Exception('Failed to create boot iso image.')

    return iso_path

def create_fs_image():
    fs_img_path = os.path.join(os.path.dirname(args.path), 'fs.img')
    if os.path.exists(fs_img_path):
        return f'file={fs_img_path},if=none,format=raw,id=x0'

    with open(fs_img_path, 'w') as file:
        # 32MiB
        file.truncate(64 * 1024 * 1024)

    return f'file={fs_img_path},if=none,format=raw,id=x0'

if __name__ == '__main__':
    main()
