use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use glob::glob;

pub const MACHINE_ARGS: &[&str] = &["-machine", "q35,kernel-irqchip=split"];

pub const NOIOMMU_DEVICE_ARGS: &[&str] = &[
    "-device",
    "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off",
    "-device",
    "virtio-keyboard-pci,disable-legacy=on,disable-modern=off",
    "-device",
    "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off",
];

pub const IOMMU_DEVICE_ARGS: &[&str] = &[
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

#[derive(Debug, Clone, Copy)]
pub enum GrubBootProtocol {
    Multiboot,
    Multiboot2,
    Linux,
}

pub fn create_bootdev_image(
    path: PathBuf,
    grub_cfg: String,
    protocol: GrubBootProtocol,
) -> PathBuf {
    let cwd = std::env::current_dir().unwrap();
    let target_dir = path.parent().unwrap();
    let out_dir = target_dir.join("boot_device");

    // Clear or make the out dir.
    if out_dir.exists() {
        fs::remove_dir_all(&out_dir).unwrap();
    }
    fs::create_dir_all(&out_dir).unwrap();

    // Find the setup header in the build script output directory.
    let bs_out_dir = glob("target/x86_64-custom/debug/build/jinux-frame-*").unwrap();
    let header_bin = Path::new(bs_out_dir.into_iter().next().unwrap().unwrap().as_path())
        .join("out")
        .join("bin")
        .join("jinux-frame-x86-boot-setup.bin");

    let target_path = match protocol {
        GrubBootProtocol::Linux => {
            // Make the `zimage`-compatible kernel image and place it in the boot directory.
            let target_path = out_dir.join("jinuz");
            make_zimage(&target_path, &path.as_path(), &header_bin.as_path()).unwrap();
            target_path
        }
        GrubBootProtocol::Multiboot | GrubBootProtocol::Multiboot2 => path.clone(),
    };
    let target_name = target_path.file_name().unwrap().to_str().unwrap();

    // Write the grub.cfg file
    let grub_cfg_path = out_dir.join("grub.cfg");
    fs::write(&grub_cfg_path, grub_cfg).unwrap();

    // Make the boot device CDROM image.

    // Firstly use `grub-mkrescue` to generate grub.img.
    let grub_img_path = out_dir.join("grub.img");
    let mut cmd = std::process::Command::new("grub-mkimage");
    cmd.arg("--format=i386-pc")
        .arg(format!("--prefix={}", out_dir.display()))
        .arg(format!("--output={}", grub_img_path.display()));
    // A embedded config file should be used to find the real config with menuentries.
    cmd.arg("--config=build/grub/grub.cfg.embedded");
    let grub_modules = &[
        "linux",
        "boot",
        "multiboot",
        "multiboot2",
        "elf",
        "loadenv",
        "memdisk",
        "biosdisk",
        "iso9660",
        "normal",
        "loopback",
        "chain",
        "configfile",
        "halt",
        "help",
        "ls",
        "reboot",
        "echo",
        "test",
        "sleep",
        "true",
        "vbe",
        "vga",
        "video_bochs",
    ];
    for module in grub_modules {
        cmd.arg(module);
    }
    if !cmd.status().unwrap().success() {
        panic!("Failed to run `{:?}`.", cmd);
    }
    // Secondly prepend grub.img with cdboot.img.
    let cdboot_path = PathBuf::from("/usr/lib/grub/i386-pc/cdboot.img");
    let mut grub_img = fs::read(cdboot_path).unwrap();
    grub_img.append(&mut fs::read(&grub_img_path).unwrap());
    fs::write(&grub_img_path, &grub_img).unwrap();

    // Finally use the `genisoimage` command to generate the CDROM image.
    let iso_path = out_dir.join(target_name.to_string() + ".iso");
    let mut cmd = std::process::Command::new("genisoimage");
    cmd.arg("-graft-points")
        .arg("-quiet")
        .arg("-R")
        .arg("-no-emul-boot")
        .arg("-boot-info-table")
        .arg("-boot-load-size")
        .arg("4")
        .arg("-input-charset")
        .arg("utf8")
        .arg("-A")
        .arg("jinux-grub2")
        .arg("-b")
        .arg(&grub_img_path)
        .arg("-o")
        .arg(&iso_path)
        .arg(format!("boot/{}={}", target_name, target_path.display()))
        .arg(format!("boot/grub/grub.cfg={}", grub_cfg_path.display()))
        .arg(format!("boot/grub/grub.img={}", grub_img_path.display()))
        .arg("boot/initramfs.cpio.gz=regression/build/initramfs.cpio.gz")
        .arg(cwd.as_os_str());
    if !cmd.status().unwrap().success() {
        panic!("Failed to run `{:?}`.", cmd);
    }

    iso_path.into()
}

pub fn generate_grub_cfg(
    template_filename: &str,
    kcmdline: &str,
    skip_grub_menu: bool,
    protocol: GrubBootProtocol,
) -> String {
    let mut buffer = String::new();

    // Read the contents of the file.
    fs::File::open(template_filename)
        .unwrap()
        .read_to_string(&mut buffer)
        .unwrap();

    // Delete the first two lines that notes the file is a template file.
    let buffer = buffer.lines().skip(2).collect::<Vec<&str>>().join("\n");
    // Set the timout style and timeout.
    let buffer = buffer
        .replace(
            "#GRUB_TIMEOUT_STYLE#",
            if skip_grub_menu { "hidden" } else { "menu" },
        )
        .replace("#GRUB_TIMEOUT#", if skip_grub_menu { "0" } else { "1" });
    // Replace all occurrences of "#KERNEL_COMMAND_LINE#" with the desired value.
    let buffer = buffer.replace("#KERNEL_COMMAND_LINE#", kcmdline);
    // Replace the grub commands according to the protocol selected.
    let buffer = match protocol {
        GrubBootProtocol::Multiboot => buffer
            .replace("#GRUB_CMD_KERNEL#", "multiboot")
            .replace("#KERNEL_NAME#", "jinux")
            .replace("#GRUB_CMD_INITRAMFS#", "module --nounzip"),
        GrubBootProtocol::Multiboot2 => buffer
            .replace("#GRUB_CMD_KERNEL#", "multiboot2")
            .replace("#KERNEL_NAME#", "jinux")
            .replace("#GRUB_CMD_INITRAMFS#", "module2 --nounzip"),
        GrubBootProtocol::Linux => buffer
            .replace("#GRUB_CMD_KERNEL#", "linux")
            .replace("#KERNEL_NAME#", "jinuz")
            .replace("#GRUB_CMD_INITRAMFS#", "initrd"),
    };

    buffer
}

fn fill_header_field(header: &mut [u8], offset: usize, value: &[u8]) {
    let size = value.len();
    assert_eq!(
        &header[offset..offset + size],
        vec![0xABu8; size].as_slice()
    );
    header[offset..offset + size].copy_from_slice(value);
}

fn make_zimage(path: &Path, kernel_path: &Path, header_path: &Path) -> std::io::Result<()> {
    let mut header = Vec::new();
    File::open(header_path)?.read_to_end(&mut header)?;
    // Pad the header to let the payload starts with 8-byte alignment.
    header.resize((header.len() + 7) & !7, 0x00);

    let mut kernel = Vec::new();
    File::open(kernel_path)?.read_to_end(&mut kernel)?;

    let header_len = header.len();
    let kernel_len = kernel.len();

    fill_header_field(
        &mut header,
        0x248, /* payload_offset */
        &(header_len as u32).to_le_bytes(),
    );
    fill_header_field(
        &mut header,
        0x24C, /* payload_length */
        &(kernel_len as u32).to_le_bytes(),
    );

    let mut kernel_image = File::create(path)?;
    kernel_image.write_all(&header)?;
    kernel_image.write_all(&kernel)?;

    Ok(())
}
