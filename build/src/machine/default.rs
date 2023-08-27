use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

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

pub fn create_bootdev_image(path: PathBuf, kcmdline: &str) -> String {
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
        kcmdline,
    );
    fs::copy(
        "regression/build/initramfs.cpio.gz",
        "target/iso_root/boot/initramfs.cpio.gz",
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

fn generate_grub_cfg(template_filename: &str, target_filename: &str, kcmdline: &str) {
    let mut buffer = String::new();

    // Read the contents of the file
    fs::File::open(template_filename)
        .unwrap()
        .read_to_string(&mut buffer)
        .unwrap();

    // Replace all occurrences of "#KERNEL_COMMAND_LINE#" with the desired value
    let replaced_content = buffer.replace("#KERNEL_COMMAND_LINE#", kcmdline);

    // Write the modified content back to the file
    fs::File::create(target_filename)
        .unwrap()
        .write_all(replaced_content.as_bytes())
        .unwrap();
}
