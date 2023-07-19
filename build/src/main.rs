use anyhow::anyhow;
use std::{
    fs::OpenOptions,
    ops::Add,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Duration,
};

const COMMON_ARGS: &[&str] = &[
    "--no-reboot",
    "-machine",
    "q35,kernel-irqchip=split",
    "-enable-kvm",
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
];

cfg_if::cfg_if!(
    if #[cfg(feature="iommu")] {
        macro_rules! virtio_device_args {
            ($($args:tt),*) => {
                concat!($($args,)*"disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",)
            };
        }
        const OPTION_ARGS: &[&str] = &[
            "-device",
            "intel-iommu,intremap=on,device-iotlb=on",
            "-device",
            "ioh3420,id=pcie.0,chassis=1",
        ];
    } else {
        macro_rules! virtio_device_args {
            ($($args:tt),*) => {
                concat!($($args,)*"disable-legacy=on,disable-modern=off",)
            };
        }
        const OPTION_ARGS: &[&str] = &[];
    }
);

const DEVICE_ARGS: &[&str] = &[
    "-device",
    "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-device",
    virtio_device_args!("virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,"),
    "-device",
    virtio_device_args!("virtio-keyboard-pci,"),
    "-device",
    virtio_device_args!("virtio-net-pci,netdev=net01,"),
    "-netdev",
    "user,id=net01,hostfwd=tcp::30022-:22,hostfwd=tcp::30080-:8080",
    "-object",
    "filter-dump,id=filter0,netdev=net01,file=virtio-net.pcap",
];

const RUN_ARGS: &[&str] = &[];
const TEST_ARGS: &[&str] = &[];
const TEST_TIMEOUT_SECS: u64 = 30;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let kernel_binary_path = {
        let path = PathBuf::from(args.next().unwrap());
        path.canonicalize().unwrap()
    };

    call_bootloader_build_script(
        &PathBuf::from("build/grub/scripts/build-grub-image.sh"),
        &kernel_binary_path,
    )
    .unwrap();

    let kernel_iso_path = {
        let a = kernel_binary_path.parent().unwrap();
        let str = kernel_binary_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        a.join(str.add(".iso"))
    };

    let mut qemu_cmd = Command::new("qemu-system-x86_64");

    let binary_kind = runner_utils::binary_kind(&kernel_binary_path);
    let mut qemu_args = COMMON_ARGS.clone().to_vec();

    qemu_args.extend(DEVICE_ARGS.clone().to_vec().iter());
    qemu_args.extend(OPTION_ARGS.clone().to_vec().iter());
    qemu_args.push("-drive");
    let binding = create_fs_image(kernel_binary_path.as_path())?;
    qemu_args.push(binding.as_str());
    qemu_cmd.arg("-cdrom");
    qemu_cmd.arg(kernel_iso_path.to_str().unwrap());
    if binary_kind.is_test() {
        qemu_args.append(&mut TEST_ARGS.to_vec());
        qemu_cmd.args(qemu_args);
        qemu_cmd.args(args);
        println!("testing:{:?}", qemu_cmd);

        let exit_status = run_test_command(qemu_cmd)?;
        match exit_status.code() {
            Some(33) => {} // success
            other => return Err(anyhow!("Test failed (exit code: {:?})", other)),
        }
    } else {
        qemu_args.append(&mut RUN_ARGS.to_vec());
        qemu_cmd.args(qemu_args);
        qemu_cmd.args(args);
        println!("running:{:?}", qemu_cmd);

        let exit_status = qemu_cmd.status()?;
        if !exit_status.success() {
            std::process::exit(exit_status.code().unwrap_or(1));
        }
    }
    Ok(())
}

fn call_bootloader_build_script(
    script_path: &PathBuf,
    kernel_path: &PathBuf,
) -> anyhow::Result<()> {
    let mut cmd = Command::new(script_path.to_str().unwrap());
    cmd.arg(kernel_path.to_str().unwrap());
    let exit_status = cmd.status()?;
    if !exit_status.success() {
        std::process::exit(exit_status.code().unwrap_or(1));
    }
    Ok(())
}

fn create_fs_image(path: &Path) -> anyhow::Result<String> {
    let mut fs_img_path = path.parent().unwrap().to_str().unwrap().to_string();
    fs_img_path.push_str("/fs.img");
    let path = Path::new(fs_img_path.as_str());
    if path.exists() {
        return Ok(format!(
            "file={},if=none,format=raw,id=x0",
            fs_img_path.as_str()
        ));
    }
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(fs_img_path.as_str())?;
    // 32MiB
    f.set_len(64 * 1024 * 1024).unwrap();
    Ok(format!(
        "file={},if=none,format=raw,id=x0",
        fs_img_path.as_str()
    ))
}

fn run_test_command(mut cmd: Command) -> anyhow::Result<ExitStatus> {
    let status = runner_utils::run_with_timeout(&mut cmd, Duration::from_secs(TEST_TIMEOUT_SECS))?;
    Ok(status)
}
