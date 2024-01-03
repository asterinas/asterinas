// SPDX-License-Identifier: MPL-2.0

use linux_bzimage_builder::{make_bzimage, BzImageType};

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use crate::BootProtocol;

macro_rules! ovmf_prefix {
    () => {
        // There are 3 optional OVMF builds at your service in the dev image
        "/root/ovmf/release/"
        // "/root/ovmf/debug/"
        // "/usr/share/OVMF/"
    };
}

pub const MACHINE_ARGS: &[&str] = &[
    "-machine",
    "q35,kernel-irqchip=split",
    "-drive",
    concat!(
        "if=pflash,format=raw,unit=0,readonly=on,file=",
        ovmf_prefix!(),
        "OVMF_CODE.fd"
    ),
    "-drive",
    concat!(
        "if=pflash,format=raw,unit=1,file=",
        ovmf_prefix!(),
        "OVMF_VARS.fd"
    ),
];

pub const NOIOMMU_DEVICE_ARGS: &[&str] = &[
    "-device",
    "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off",
    "-device",
    "virtio-keyboard-pci,disable-legacy=on,disable-modern=off",
    "-device",
    "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off",
    "-device",
    "virtio-serial-pci,disable-legacy=on,disable-modern=off",
    "-device",
    "virtconsole,chardev=mux",
];

pub const IOMMU_DEVICE_ARGS: &[&str] = &[
    "-device",
    "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",
    "-device",
    "virtio-keyboard-pci,disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",
    "-device",
    "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",
    "-device",
    "virtio-serial-pci,disable-legacy=on,disable-modern=off,iommu_platform=on,ats=on",
    "-device",
    "virtconsole,chardev=mux",
    "-device",
    "intel-iommu,intremap=on,device-iotlb=on",
    "-device",
    "ioh3420,id=pcie.0,chassis=1",
];

/// The default GRUB tools used.
pub const GRUB_PREFIX: &str = "/usr";
/// The GRUB version that defaults to use EFI handover. Which is the Debian APT version.
pub const GRUB_PREFIX_EFI_HANDOVER: &str = "/usr";
/// The GRUB version that uses Loadfile2 and has a fallback to use legacy boot. Which is the custom built upstream 2.12 verion.
pub const GRUB_PREFIX_EFI_AND_LEGACY: &str = "/usr/local/grub";

pub const GRUB_VERSION: &str = "x86_64-efi";

pub fn create_bootdev_image(
    kernel_elf_path: PathBuf,
    initramfs_path: PathBuf,
    grub_cfg: String,
    protocol: BootProtocol,
) -> PathBuf {
    let target_dir = kernel_elf_path.parent().unwrap();
    let iso_root = target_dir.join("iso_root");

    // Clear or make the iso dir.
    if iso_root.exists() {
        fs::remove_dir_all(&iso_root).unwrap();
    }
    fs::create_dir_all(iso_root.join("boot").join("grub")).unwrap();

    // Copy the initramfs to the boot directory.
    fs::copy(
        initramfs_path,
        iso_root.join("boot").join("initramfs.cpio.gz"),
    )
    .unwrap();

    let target_path = match protocol {
        BootProtocol::LinuxLegacy32 | BootProtocol::LinuxEfiHandover64 => {
            let image_type = match protocol {
                BootProtocol::LinuxLegacy32 => BzImageType::Legacy32,
                BootProtocol::LinuxEfiHandover64 => BzImageType::Efi64,
                _ => unreachable!(),
            };
            let setup_src = Path::new("framework/libs/linux-bzimage/setup");
            let setup_out_dir = Path::new("target/linux-bzimage-setup");
            // Make the `bzImage`-compatible kernel image and place it in the boot directory.
            let target_path = iso_root.join("boot").join("asterinaz");
            println!("[aster-runner] Building bzImage.");
            make_bzimage(
                &target_path,
                image_type,
                &kernel_elf_path.as_path(),
                &setup_src,
                &setup_out_dir,
            );
            target_path
        }
        BootProtocol::Multiboot | BootProtocol::Multiboot2 => {
            // Copy the kernel image to the boot directory.
            let target_path = iso_root.join("boot").join("atserinas");
            fs::copy(&kernel_elf_path, &target_path).unwrap();
            target_path
        }
    };
    let target_name = target_path.file_name().unwrap().to_str().unwrap();

    // Write the grub.cfg file
    let grub_cfg_path = iso_root.join("boot").join("grub").join("grub.cfg");
    fs::write(&grub_cfg_path, grub_cfg).unwrap();

    // Make the boot device CDROM image.
    let iso_path = target_dir.join(target_name.to_string() + ".iso");
    let grub_mkrescue_bin = match protocol {
        BootProtocol::LinuxLegacy32 => PathBuf::from(GRUB_PREFIX_EFI_AND_LEGACY),
        BootProtocol::LinuxEfiHandover64 => PathBuf::from(GRUB_PREFIX_EFI_HANDOVER),
        BootProtocol::Multiboot | BootProtocol::Multiboot2 => PathBuf::from(GRUB_PREFIX),
    }
    .join("bin")
    .join("grub-mkrescue");
    let mut cmd = std::process::Command::new(grub_mkrescue_bin.as_os_str());
    cmd.arg("--output").arg(&iso_path).arg(iso_root.as_os_str());
    if !cmd.status().unwrap().success() {
        panic!("Failed to run `{:?}`.", cmd);
    }

    iso_path.into()
}

pub fn generate_grub_cfg(
    template_filename: &str,
    kcmdline: &str,
    skip_grub_menu: bool,
    protocol: BootProtocol,
) -> String {
    let mut buffer = String::new();

    // Read the contents of the file.
    fs::File::open(template_filename)
        .unwrap()
        .read_to_string(&mut buffer)
        .unwrap();

    // Delete the first two lines that notes the file a template file.
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
        BootProtocol::Multiboot => buffer
            .replace("#GRUB_CMD_KERNEL#", "multiboot")
            .replace("#KERNEL#", "/boot/atserinas")
            .replace("#GRUB_CMD_INITRAMFS#", "module --nounzip"),
        BootProtocol::Multiboot2 => buffer
            .replace("#GRUB_CMD_KERNEL#", "multiboot2")
            .replace("#KERNEL#", "/boot/atserinas")
            .replace("#GRUB_CMD_INITRAMFS#", "module2 --nounzip"),
        BootProtocol::LinuxLegacy32 | BootProtocol::LinuxEfiHandover64 => buffer
            .replace("#GRUB_CMD_KERNEL#", "linux")
            .replace("#KERNEL#", "/boot/asterinaz")
            .replace("#GRUB_CMD_INITRAMFS#", "initrd"),
    };

    buffer
}
