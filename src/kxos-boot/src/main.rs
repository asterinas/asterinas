use anyhow::anyhow;
use std::{
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Duration,
};
const RUN_ARGS: &[&str] = &["--no-reboot", "-s"];
const TEST_ARGS: &[&str] = &[
    "-device",
    "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-serial",
    "stdio",
    "-display",
    "none",
    "--no-reboot",
];
const TEST_TIMEOUT_SECS: u64 = 10;
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1); // skip executable name
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

    let bios = create_disk_images(&kernel_binary_path);

    if no_boot {
        println!("Created disk image at `{}`", bios.display());
        return Ok(());
    }

    let mut run_cmd = Command::new("qemu-system-x86_64");
    run_cmd
        .arg("-drive")
        .arg(format!("format=raw,file={}", bios.display()));

    let binary_kind = runner_utils::binary_kind(&kernel_binary_path);
    if binary_kind.is_test() {
        run_cmd.args(TEST_ARGS);

        let exit_status = run_test_command(run_cmd)?;
        match exit_status.code() {
            Some(33) => {} // success
            other => return Err(anyhow!("Test failed (exit code: {:?})", other)),
        }
    } else {
        run_cmd.args(RUN_ARGS);

        let exit_status = run_cmd.status()?;
        if !exit_status.success() {
            std::process::exit(exit_status.code().unwrap_or(1));
        }
    }
    Ok(())
}

pub fn create_disk_images(kernel_binary_path: &Path) -> PathBuf {
    let bootloader_manifest_path = bootloader_locator::locate_bootloader("bootloader").unwrap();
    let kernel_manifest_path = locate_cargo_manifest::locate_manifest().unwrap();

    let mut build_cmd = Command::new(env!("CARGO"));
    build_cmd.current_dir(bootloader_manifest_path.parent().unwrap());
    build_cmd.arg("builder");
    build_cmd
        .arg("--kernel-manifest")
        .arg(&kernel_manifest_path);
    build_cmd.arg("--kernel-binary").arg(&kernel_binary_path);
    build_cmd
        .arg("--target-dir")
        .arg(kernel_manifest_path.parent().unwrap().join("target"));
    build_cmd
        .arg("--out-dir")
        .arg(kernel_binary_path.parent().unwrap());
    build_cmd.arg("--quiet");

    if !build_cmd.status().unwrap().success() {
        panic!("build failed");
    }

    let kernel_binary_name = kernel_binary_path.file_name().unwrap().to_str().unwrap();
    let disk_image = kernel_binary_path
        .parent()
        .unwrap()
        .join(format!("boot-bios-{}.img", kernel_binary_name));
    if !disk_image.exists() {
        panic!(
            "Disk image does not exist at {} after bootloader build",
            disk_image.display()
        );
    }
    disk_image
}

fn run_test_command(mut cmd: Command) -> anyhow::Result<ExitStatus> {
    let status = runner_utils::run_with_timeout(&mut cmd, Duration::from_secs(TEST_TIMEOUT_SECS))?;
    Ok(status)
}
