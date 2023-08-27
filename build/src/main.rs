//! jinux-build is the Jinux runner script to ease the pain of running
//! and testing Jinux inside a QEMU VM, which should be called as the
//! cargo runner: https://doc.rust-lang.org/cargo/reference/config.html
//!
//! The runner generates the the filesystem image and the containing
//! boot device image. Then it invokes QEMU to boot Jinux.
//!

pub mod machine;

use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::Command,
};

use clap::Parser;

use crate::machine::{default, microvm};

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

    /// Run Jinux as microvm mode.
    #[arg(long, default_value_t = false)]
    run_microvm: bool,
}

pub const COMMON_ARGS: &[&str] = &[
    "--no-reboot",
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

fn main() {
    let args = Args::parse();

    let mut qemu_cmd = Command::new("qemu-system-x86_64");

    let mut qemu_args = COMMON_ARGS.to_vec();

    if args.enable_kvm {
        qemu_args.push("-enable-kvm");
    }
    // Specify machine type
    if args.run_microvm {
        qemu_args.extend(microvm::MACHINE_ARGS.to_vec().iter());
    } else {
        qemu_args.extend(default::MACHINE_ARGS.to_vec().iter());
    }
    // Add device arguments
    if args.run_microvm {
        qemu_args.extend(microvm::DEVICE_ARGS.to_vec().iter())
    } else if args.emulate_iommu {
        qemu_args.extend(default::IOMMU_DEVICE_ARGS.to_vec().iter());
    } else {
        qemu_args.extend(default::NOIOMMU_DEVICE_ARGS.to_vec().iter());
    }

    let fs_image = create_fs_image(args.path.as_path());
    qemu_args.push("-drive");
    qemu_args.push(fs_image.as_str());

    if args.run_microvm {
        let image = microvm::create_bootdev_image(args.path);
        qemu_cmd.arg("-kernel");
        qemu_cmd.arg(image.as_str());
        qemu_cmd.arg("-append");
        qemu_cmd.arg(&args.kcmdline);
        qemu_cmd.arg("-initrd");
        qemu_cmd.arg("regression/build/initramfs.cpio.gz");
    } else {
        let bootdev_image = default::create_bootdev_image(args.path, &args.kcmdline);
        qemu_cmd.arg("-cdrom");
        qemu_cmd.arg(bootdev_image.as_str());
    }

    qemu_cmd.args(qemu_args);

    println!("running:{:#?}", qemu_cmd);

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

pub fn create_fs_image(path: &Path) -> String {
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
