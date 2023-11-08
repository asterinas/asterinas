//! jinux-runner is the Jinux runner script to ease the pain of running
//! and testing Jinux inside a QEMU VM. It should be built and run as the
//! cargo runner: https://doc.rust-lang.org/cargo/reference/config.html
//!
//! The runner will generate the filesystem image for starting Jinux. If
//! we should use the runner in the default mode, which invokes QEMU with
//! a GRUB boot device image, the runner would be responsible for generating
//! the appropriate kernel image and the boot device image. It also supports
//! to directly boot the kernel image without GRUB using the QEMU microvm
//! machine type.
//!

pub mod gdb;
pub mod machine;

use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Parser, ValueEnum};

use crate::machine::{microvm, qemu_grub_efi};

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
enum BootMethod {
    QemuGrub,
    Microvm,
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum BootProtocol {
    Multiboot,
    Multiboot2,
    Linux,
}
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
    /// Boot method. Can be one of the following items:
    ///  - `qemu-grub`;
    ///  - `microvm`.
    #[arg(long, value_enum, default_value_t = BootMethod::QemuGrub)]
    boot_method: BootMethod,

    /// Boot protocol. Can be one of the following items:
    ///  - `multiboot`;
    ///  - `multiboot2`;
    ///  - `linux`.
    #[arg(long, value_enum, default_value_t = BootProtocol::Multiboot2)]
    boot_protocol: BootProtocol,

    /// Enable KVM when running QEMU.
    #[arg(long, default_value_t = false)]
    enable_kvm: bool,

    /// Emulate Intel IOMMU by QEMU.
    #[arg(long, default_value_t = false)]
    emulate_iommu: bool,

    /// Run QEMU as a GDB server.
    #[arg(long, default_value_t = false)]
    halt_for_gdb: bool,

    /// Boot without displaying the GRUB menu.
    #[arg(long, default_value_t = false)]
    skip_grub_menu: bool,

    /// Run a GDB client instead of running the kernel.
    #[arg(long, default_value_t = false)]
    run_gdb_client: bool,

    /// Run in the release mode.
    #[arg(long, default_value_t = false)]
    release_mode: bool,
}

pub const COMMON_ARGS: &[&str] = &[
    "--no-reboot",
    "-cpu",
    "Icelake-Server,+x2apic",
    "-m",
    "2G",
    "-nographic", // TODO: figure out why grub can't shown up without it
    "-serial",
    "chardev:mux",
    "-monitor",
    "chardev:mux",
    "-chardev",
    "stdio,id=mux,mux=on,signal=off,logfile=qemu.log",
    "-display",
    "none",
    "-device",
    "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-object",
    "filter-dump,id=filter0,netdev=net01,file=virtio-net.pcap",
];

pub fn random_hostfwd_ports() -> (u16, u16) {
    let start = 32768u16;
    let end = 61000u16;
    let port1 = rand::random::<u16>() % (end - 1 - start) + start;
    let port2 = rand::random::<u16>() % (end - port1) + port1;
    (port1, port2)
}

pub const GDB_ARGS: &[&str] = &[
    "-chardev",
    "socket,path=/tmp/jinux-gdb-socket,server=on,wait=off,id=gdb0",
    "-gdb",
    "chardev:gdb0",
    "-S",
];

fn main() {
    let args = Args::parse();
    if args.run_gdb_client {
        let gdb_grub = args.boot_method == BootMethod::QemuGrub;
        // You should comment out the next line if you want to debug grub instead
        // of the kernel because this argument is not exposed by runner CLI.
        let gdb_grub = gdb_grub && false;
        gdb::run_gdb_client(&args.path, gdb_grub);
        return;
    }

    let mut qemu_cmd = Command::new("qemu-system-x86_64");

    qemu_cmd.args(COMMON_ARGS);

    qemu_cmd.arg("-netdev");
    let (port1, port2) = random_hostfwd_ports();
    qemu_cmd.arg(format!(
        "user,id=net01,hostfwd=tcp::{}-:22,hostfwd=tcp::{}-:8080",
        port1, port2
    ));
    println!(
        "[jinux-runner] Binding host ports to guest ports: ({} -> {}); ({} -> {}).",
        port1, 22, port2, 8080
    );

    if args.halt_for_gdb {
        if args.enable_kvm {
            println!("[jinux-runner] Can't enable KVM when running QEMU as a GDB server. Abort.");
            return;
        }
        qemu_cmd.args(GDB_ARGS);
    }

    if args.enable_kvm {
        qemu_cmd.arg("-enable-kvm");
    }
    // Add machine-specific arguments
    if args.boot_method == BootMethod::QemuGrub {
        qemu_cmd.args(qemu_grub_efi::MACHINE_ARGS);
    } else if args.boot_method == BootMethod::Microvm {
        qemu_cmd.args(microvm::MACHINE_ARGS);
    }
    // Add device arguments
    if args.boot_method == BootMethod::Microvm {
        qemu_cmd.args(microvm::DEVICE_ARGS);
    } else if args.emulate_iommu {
        qemu_cmd.args(qemu_grub_efi::IOMMU_DEVICE_ARGS);
    } else {
        qemu_cmd.args(qemu_grub_efi::NOIOMMU_DEVICE_ARGS);
    }

    let fs_image = create_fs_image(args.path.as_path());
    qemu_cmd.arg("-drive");
    qemu_cmd.arg(fs_image);

    if args.boot_method == BootMethod::Microvm {
        let image = microvm::create_bootdev_image(args.path);
        qemu_cmd.arg("-kernel");
        qemu_cmd.arg(image.as_os_str());
        qemu_cmd.arg("-append");
        qemu_cmd.arg(&args.kcmdline);
        qemu_cmd.arg("-initrd");
        qemu_cmd.arg("regression/build/initramfs.cpio.gz");
    } else if args.boot_method == BootMethod::QemuGrub {
        let grub_cfg = qemu_grub_efi::generate_grub_cfg(
            "runner/grub/grub.cfg.template",
            &args.kcmdline,
            args.skip_grub_menu,
            args.boot_protocol,
        );
        let initramfs_path = PathBuf::from("regression/build/initramfs.cpio.gz");
        let bootdev_image = qemu_grub_efi::create_bootdev_image(
            args.path,
            initramfs_path,
            grub_cfg,
            args.boot_protocol,
            args.release_mode,
        );
        qemu_cmd.arg("-cdrom");
        qemu_cmd.arg(bootdev_image.as_os_str());
    }

    println!("[jinux-runner] Running: {:#?}", qemu_cmd);

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
