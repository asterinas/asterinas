use anyhow::anyhow;
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Duration,
};
const COMMON_ARGS: &[&str] = &[
    "--no-reboot",
    "-cpu",
    "Icelake-Server",
    "-device",
    "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-device",
    "virtio-blk-pci,bus=pci.0,addr=0x6,drive=x0",
    "-device",
    "virtio-keyboard-pci",
    "-serial",
    "mon:stdio",
    "-display",
    "none",
];

const RUN_ARGS: &[&str] = &["-s"];
const TEST_ARGS: &[&str] = &[];
const TEST_TIMEOUT_SECS: u64 = 10;
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let kernel_binary_path = {
        let path = PathBuf::from(args.next().unwrap());
        path.canonicalize().unwrap()
    };

    let no_boot = if let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-run" => true,
            other => panic!("unexpected argument `{}`", other),
        }
    } else {
        false
    };

    #[cfg(feature = "limine")]
    call_limine_build_script(&kernel_binary_path).unwrap();
    // add .iso

    let kernel_iso_path = {
        let a = kernel_binary_path.parent().unwrap();
        a.join("jinux.iso")
    };
    // let bios = create_disk_images(&kernel_binary_path);

    if no_boot {
        println!("Created disk image at `{}`", kernel_iso_path.display());
        return Ok(());
    }
    #[cfg(windows)]
    let mut run_cmd = Command::new("qemu-system-x86_64.exe");
    #[cfg(not(windows))]
    let mut run_cmd = Command::new("qemu-system-x86_64");

    let binary_kind = runner_utils::binary_kind(&kernel_binary_path);
    let mut args = COMMON_ARGS.clone().to_vec();
    args.push("-drive");
    let binding = create_fs_image(kernel_binary_path.as_path())?;
    args.push(binding.as_str());
    run_cmd.arg(kernel_iso_path.to_str().unwrap());
    if binary_kind.is_test() {
        args.append(&mut TEST_ARGS.to_vec());
        run_cmd.args(args);

        let exit_status = run_test_command(run_cmd)?;
        match exit_status.code() {
            Some(33) => {} // success
            other => return Err(anyhow!("Test failed (exit code: {:?})", other)),
        }
    } else {
        args.append(&mut RUN_ARGS.to_vec());
        run_cmd.args(args);

        let exit_status = run_cmd.status()?;
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
