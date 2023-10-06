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
    let dir = path.parent().unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_string();
    let iso_path = dir.join(name + ".iso").to_str().unwrap().to_string();

    // Clean up the image directory.
    if Path::new("target/iso_root").exists() {
        fs::remove_dir_all("target/iso_root").unwrap();
    }

    // Copy the needed files into an ISO image.
    fs::create_dir_all("target/iso_root/boot/grub").unwrap();

    fs::copy(
        "regression/build/initramfs.cpio.gz",
        "target/iso_root/boot/initramfs.cpio.gz",
    )
    .unwrap();

    // Find the setup header in the build script output directory.
    let out_dir = glob("target/x86_64-custom/debug/build/jinux-frame-*").unwrap();
    let header_bin = Path::new(out_dir.into_iter().next().unwrap().unwrap().as_path())
        .join("out")
        .join("bin")
        .join("jinux-frame-x86-boot-setup.bin");

    // Deliver the kernel image to the boot directory.
    match protocol {
        GrubBootProtocol::Linux => {
            // Make the `zimage`-compatible kernel image and place it in the boot directory.
            make_zimage(
                &Path::new("target/iso_root/boot/jinux"),
                &path.as_path(),
                &header_bin.as_path(),
            )
            .unwrap();
        }
        GrubBootProtocol::Multiboot | GrubBootProtocol::Multiboot2 => {
            // Copy the kernel image into the boot directory.
            fs::copy(&path, "target/iso_root/boot/jinux").unwrap();
        }
    }

    // Write the grub.cfg file
    fs::write("target/iso_root/boot/grub/grub.cfg", grub_cfg).unwrap();

    // Make the boot device .iso image.
    let status = std::process::Command::new("grub-mkrescue")
        .arg("-o")
        .arg(&iso_path)
        .arg("target/iso_root")
        .status()
        .unwrap();

    if !status.success() {
        panic!("Failed to create boot iso image.")
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
            .replace("#GRUB_CMD_INITRAMFS#", "module --nounzip"),
        GrubBootProtocol::Multiboot2 => buffer
            .replace("#GRUB_CMD_KERNEL#", "multiboot2")
            .replace("#GRUB_CMD_INITRAMFS#", "module2 --nounzip"),
        GrubBootProtocol::Linux => buffer
            .replace("#GRUB_CMD_KERNEL#", "linux")
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
