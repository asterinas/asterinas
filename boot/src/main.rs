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
    "-m",
    "2G",
    "-cpu",
    "Icelake-Server",
    "-m",
    "1G",
    "-device",
    "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-device",
    "virtio-blk-pci,bus=pci.0,addr=0x6,drive=x0",
    "-device",
    "virtio-keyboard-pci",
    "-monitor",
    "vc",
    "-serial",
    "mon:stdio",
    "-display",
    "none",
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

    #[cfg(feature = "limine")]
    call_limine_build_script(&kernel_binary_path).unwrap();
    // add .iso

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

    #[cfg(windows)]
    let mut qemu_cmd = Command::new("qemu-system-x86_64.exe");
    #[cfg(not(windows))]
    let mut qemu_cmd = Command::new("qemu-system-x86_64");

    let binary_kind = runner_utils::binary_kind(&kernel_binary_path);
    let mut qemu_args = COMMON_ARGS.clone().to_vec();
    qemu_args.push("-drive");
    let binding = create_fs_image(kernel_binary_path.as_path())?;
    qemu_args.push(binding.as_str());
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

fn call_limine_build_script(path: &PathBuf) -> anyhow::Result<()> {
    let mut cmd = Command::new("boot/limine/scripts/limine-build.sh");
    cmd.arg(path.to_str().unwrap());
    let exit_status = cmd.status()?;
    if !exit_status.success() {
        std::process::exit(exit_status.code().unwrap_or(1));
    }
    Ok(())
}

fn create_fs_image(path: &Path) -> anyhow::Result<String> {
    let mut fs_img_path = path.parent().unwrap().to_str().unwrap().to_string();
    #[cfg(windows)]
    fs_img_path.push_str("\\fs.img");
    #[cfg(not(windows))]
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
