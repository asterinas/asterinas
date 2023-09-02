//! jinux-build is the Jinux runner script to ease the pain of running
//! and testing Jinux inside a QEMU VM, which should be called as the
//! cargo runner: https://doc.rust-lang.org/cargo/reference/config.html
//!
//! The runner generates the the filesystem image and the containing
//! boot device image. Then it invokes QEMU to boot Jinux.
//!

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use clap::{builder::Str, Parser};

/// The CLI of this runner.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // Positional arguments.
    /// The Jinux binary path.
    path: PathBuf,

    /// Provide the kernel commandline, which specifies
    /// the init process.
    kcmdline: String,

    // Optional arguments.
    /// Enable KVM when running QEMU.
    #[arg(long, default_value_t = false)]
    enable_kvm: bool,

    /// Emulate Intel IOMMU by QEMU.
    #[arg(long, default_value_t = false)]
    emulate_iommu: bool,
}

const COMMON_ARGS: &[&str] = &[
    "--no-reboot",
    "-machine",
    "q35,kernel-irqchip=split",
    "-cpu",
    "Icelake-Server,+x2apic",
    "-m",
    "2G",
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
    "user,id=net01,hostfwd=tcp::30133-:22,hostfwd=tcp::31088-:8080",
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

    let mut qemu_cmd = Command::new("qemu-system-x86_64");

    let mut qemu_args = COMMON_ARGS.clone().to_vec();
    if args.enable_kvm {
        qemu_args.push("-enable-kvm");
    }
    if args.emulate_iommu {
        qemu_args.extend(IOMMU_DEVICE_ARGS.clone().to_vec().iter());
    } else {
        qemu_args.extend(COMMON_DEVICE_ARGS.clone().to_vec().iter());
    }

    let fs_image = create_fs_image(args.path.as_path());
    qemu_args.push("-drive");
    qemu_args.push(fs_image.as_str());

    let bootdev_image = create_bootdev_image(args.path, &args.kcmdline);
    qemu_cmd.arg("-cdrom");
    qemu_cmd.arg(bootdev_image.as_str());

    qemu_cmd.args(qemu_args);

    println!("running:{:?}", qemu_cmd);

    let exit_status = qemu_cmd.status().unwrap();
    if !exit_status.success() {
        // FIXME: Exit code manipulation is not needed when using non-x86 QEMU
        let qemu_exit_code = exit_status.code().unwrap();
        let kernel_exit_code = qemu_exit_code >> 1;
        match kernel_exit_code {
            0x10 /*jinux_frame::QemuExitCode::Success*/ => { std::process::exit(0); },
            0x20 /*jinux_frame::QemuExitCode::Failed*/ => { std::process::exit(1); },
            _ => { std::process::exit(qemu_exit_code) },
        }
    }
}

fn generate_grub_cfg(template_filename: &str, target_filename: &str, kcmdline: &str) {
    let mut buffer = String::new();

    // Read the contents of the file
    fs::File::open(template_filename)
        .unwrap()
        .read_to_string(&mut buffer)
        .unwrap();

    // Replace all occurrences of "#KERNEL_COMMAND_LINE#" with the desired value
    let replaced_content = buffer.replace("#KERNEL_COMMAND_LINE#", kcmdline);

    // Write the modified content back to the file
    fs::File::create(target_filename)
        .unwrap()
        .write_all(replaced_content.as_bytes())
        .unwrap();
}

fn create_bootdev_image(path: PathBuf, kcmdline: &str) -> String {
    let dir = path.parent().unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_string();
    let iso_path = dir.join(name + ".iso").to_str().unwrap().to_string();

    // Clean up the image directory
    if Path::new("target/iso_root").exists() {
        fs::remove_dir_all("target/iso_root").unwrap();
    }

    // Copy the needed files into an ISO image.
    fs::create_dir_all("target/iso_root/boot/grub").unwrap();

    fs::copy(path.as_os_str(), "target/iso_root/boot/jinux").unwrap();
    generate_grub_cfg(
        "build/grub/grub.cfg.template",
        "target/iso_root/boot/grub/grub.cfg",
        kcmdline,
    );
    fs::copy(
        "regression/build/initramfs.cpio.gz",
        "target/iso_root/boot/initramfs.cpio.gz",
    )
    .unwrap();

    // Make the boot device .iso image
    let status = std::process::Command::new("grub-mkrescue")
        .arg("-o")
        .arg(&iso_path)
        .arg("target/iso_root")
        .status()
        .unwrap();

    if !status.success() {
        panic!("Failed to create boot iso image.")
    }

    iso_path
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
