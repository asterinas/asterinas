//! This is the Jinux runner script (I repeat: script) to ease the pain of
//! running and testing Jinux inside a QEMU VM.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
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
    syscall_test: bool,

    /// Enable KVM when running QEMU.
    #[arg(short, long, default_value_t = false)]
    enable_kvm: bool,

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

    let mut qemu_cmd = Command::new("qemu-system-x86_64");

    let mut qemu_args = COMMON_ARGS.clone().to_vec();
    if args.enable_kvm {
        qemu_args.push("-enable-kvm");
    }
    if args.iommu {
        qemu_args.extend(IOMMU_DEVICE_ARGS.clone().to_vec().iter());
    } else {
        qemu_args.extend(COMMON_DEVICE_ARGS.clone().to_vec().iter());
    }

    let fs_image = create_fs_image(args.path.as_path());
    qemu_args.push("-drive");
    qemu_args.push(fs_image.as_str());

    let bootdev_image = create_bootdev_image(args.path, args.syscall_test);
    qemu_cmd.arg("-cdrom");
    qemu_cmd.arg(bootdev_image.as_str());

    qemu_cmd.args(qemu_args);

    println!("running:{:?}", qemu_cmd);

    let exit_status = qemu_cmd.status().unwrap();
    if !exit_status.success() {
        std::process::exit(exit_status.code().unwrap_or(1));
    }
}

const KERNEL_CMDLINE: &str =
    r#"SHELL="/bin/sh" LOGNAME="root" HOME="/" USER="root" PATH="/bin" init=/usr/bin/busybox"#;
const EVAL_INIT_CMDLINE: &str = r#"sh -l /opt/syscall_test/run_syscall_test.sh"#;
const COMMON_INIT_CMDLINE: &str = r#"sh -l"#;

fn generate_grub_cfg(template_filename: &str, target_filename: &str, is_eval: bool) {
    let mut buffer = String::new();

    // Read the contents of the file
    fs::File::open(template_filename)
        .unwrap()
        .read_to_string(&mut buffer)
        .unwrap();

    // Replace all occurrences of "#KERNEL_COMMAND_LINE#" with the desired value
    let cmdline = if is_eval {
        KERNEL_CMDLINE.to_string() + " -- " + EVAL_INIT_CMDLINE
    } else {
        KERNEL_CMDLINE.to_string() + " -- " + COMMON_INIT_CMDLINE
    };
    let replaced_content = buffer.replace("#KERNEL_COMMAND_LINE#", &cmdline);

    // Write the modified content back to the file
    fs::File::create(target_filename)
        .unwrap()
        .write_all(replaced_content.as_bytes())
        .unwrap();
}

fn create_bootdev_image(path: PathBuf, is_eval: bool) -> String {
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
        is_eval,
    );
    fs::copy(
        "regression/build/ramdisk.cpio.gz",
        "target/iso_root/boot/ramdisk.cpio.gz",
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
