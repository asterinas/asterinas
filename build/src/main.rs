//! This is the Jinux runner to ease the pain of
//! running and testing Jinux inside a QEMU VM.

use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::Command,
};

use clap::Parser;

/// The CLI of this runner.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // Positional arguments.
    /// The Jinux binary path.
    path: PathBuf,

    // Options.
    /// Automatically run integration tests and exit.
    #[arg(short, long, default_value_t = false)]
    eval: bool,

    /// Emulate Intel IOMMU by QEMU.
    #[arg(short, long, default_value_t = false)]
    iommu: bool,
}

const COMMON_ARGS: &[&str] = &[
    "--no-reboot",
    "-machine",
    "q35,kernel-irqchip=split",
    "-cpu",
    "Icelake-Server,+x2apic",
    "-m",
    "2G",
    "-enable-kvm",
    "-nographic", // TODO: figure out why grub can't shown up without it
    "-monitor",
    "vc",
    "-serial",
    "mon:stdio",
    "-display",
    "none",
    "-device",
    "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-netdev",
    "user,id=net01,hostfwd=tcp::30022-:22,hostfwd=tcp::30080-:8080",
    "-object",
    "filter-dump,id=filter0,netdev=net01,file=virtio-net.pcap",
];

const COMMON_DEVICE_ARGS: &[&str] = &[
    "-device",
    "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off",
    "-device",
    "virtio-keyboard-pci,disable-legacy=on,disable-modern=off",
    "-device",
    "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off",
];

const IOMMU_DEVICE_ARGS: &[&str] = &[
    "-device",
    "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",
    "-device",
    "virtio-keyboard-pci,disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",
    "-device",
    "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",
    "-device",
    "intel-iommu,intremap=on,device-iotlb=on",
    "-device",
    "ioh3420,id=pcie.0,chassis=1",
];

fn main() {
    let args = Args::parse();

    if args.eval {
        panic!("No eval yet.");
    }

    let mut qemu_cmd = Command::new("qemu-system-x86_64");

    let mut qemu_args = COMMON_ARGS.clone().to_vec();
    if args.iommu {
        qemu_args.extend(IOMMU_DEVICE_ARGS.clone().to_vec().iter());
    } else {
        qemu_args.extend(COMMON_DEVICE_ARGS.clone().to_vec().iter());
    }

    let fs_image = create_fs_image(args.path.as_path());
    qemu_args.push("-drive");
    qemu_args.push(fs_image.as_str());

    let bootdev_image = create_bootdev_image(args.path);
    qemu_cmd.arg("-cdrom");
    qemu_cmd.arg(bootdev_image.as_str());

    qemu_cmd.args(qemu_args);

    println!("running:{:?}", qemu_cmd);

    let exit_status = qemu_cmd.status().unwrap();
    if !exit_status.success() {
        std::process::exit(exit_status.code().unwrap_or(1));
    }
}

fn call_bootloader_build_script(script_path: &PathBuf, kernel_path: &PathBuf) {
    let mut cmd = Command::new(script_path.to_str().unwrap());
    cmd.arg(kernel_path.to_str().unwrap());
    let exit_status = cmd.status().unwrap();
    if !exit_status.success() {
        std::process::exit(exit_status.code().unwrap_or(1));
    }
}

fn create_bootdev_image(path: PathBuf) -> String {
    call_bootloader_build_script(
        &PathBuf::from("build/grub/scripts/build-grub-image.sh"),
        &path,
    );
    let dir = path.parent().unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_string();
    dir.join(name + ".iso").to_str().unwrap().to_string()
}

fn create_fs_image(path: &Path) -> String {
    let mut fs_img_path = path.parent().unwrap().to_str().unwrap().to_string();
    fs_img_path.push_str("/fs.img");
    let path = Path::new(fs_img_path.as_str());
    if path.exists() {
        return format!("file={},if=none,format=raw,id=x0", fs_img_path.as_str());
    }
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(fs_img_path.as_str())
        .unwrap();
    // 32MiB
    f.set_len(64 * 1024 * 1024).unwrap();
    format!("file={},if=none,format=raw,id=x0", fs_img_path.as_str())
}
