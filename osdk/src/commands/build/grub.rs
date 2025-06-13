// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::bin::make_install_bzimage;
use crate::{
    bundle::{
        bin::AsterBin,
        file::BundleFile,
        vm_image::{AsterGrubIsoImageMeta, AsterVmImage, AsterVmImageType},
    },
    config::{
        scheme::{ActionChoice, BootProtocol},
        Config,
    },
    util::{get_current_crates, hard_link_or_copy, new_command_checked_exists},
};

pub fn create_bootdev_image(
    target_dir: impl AsRef<Path>,
    aster_bin: &AsterBin,
    initramfs_path: Option<impl AsRef<Path>>,
    config: &Config,
    action: ActionChoice,
) -> AsterVmImage {
    let target_name = get_current_crates().remove(0).name;
    let iso_root = &target_dir.as_ref().join("iso_root");
    let action = match &action {
        ActionChoice::Run => &config.run,
        ActionChoice::Test => &config.test,
    };
    let protocol = &action.grub.boot_protocol;

    // Clear or make the iso dir.
    if iso_root.exists() {
        fs::remove_dir_all(iso_root).unwrap();
    }
    fs::create_dir_all(iso_root.join("boot").join("grub")).unwrap();

    // Copy the initramfs to the boot directory.
    if let Some(init_path) = &initramfs_path {
        hard_link_or_copy(
            init_path.as_ref().to_str().unwrap(),
            iso_root.join("boot").join("initramfs.cpio.gz"),
        )
        .unwrap();
    }

    // Make the kernel image and place it in the boot directory.
    match protocol {
        BootProtocol::Linux => {
            make_install_bzimage(
                iso_root.join("boot"),
                &target_dir,
                aster_bin,
                action.build.linux_x86_legacy_boot,
                config.build.encoding.clone(),
            );
        }
        _ => {
            // Copy the kernel image to the boot directory.
            let target_path = iso_root.join("boot").join(&target_name);
            hard_link_or_copy(aster_bin.path(), target_path).unwrap();
        }
    };

    // Write the grub.cfg file
    let initramfs_in_image = if initramfs_path.is_some() {
        Some("/boot/initramfs.cpio.gz".to_string())
    } else {
        None
    };
    let grub_cfg = generate_grub_cfg(
        &action.boot.kcmdline.join(" "),
        !action.grub.display_grub_menu,
        initramfs_in_image,
        protocol,
    );
    let grub_cfg_path = iso_root.join("boot").join("grub").join("grub.cfg");
    fs::write(grub_cfg_path, grub_cfg).unwrap();

    // Make the boot device CDROM image using `grub-mkrescue`.
    let iso_path = &target_dir.as_ref().join(target_name.to_string() + ".iso");
    let mut grub_mkrescue_cmd = new_command_checked_exists(action.grub.grub_mkrescue.as_os_str());
    grub_mkrescue_cmd
        .arg(iso_root.as_os_str())
        .arg("-o")
        .arg(iso_path);
    if !grub_mkrescue_cmd.status().unwrap().success() {
        panic!("Failed to run {:#?}.", grub_mkrescue_cmd);
    }

    AsterVmImage::new(
        iso_path,
        AsterVmImageType::GrubIso(AsterGrubIsoImageMeta {
            grub_version: get_grub_mkrescue_version(&action.grub.grub_mkrescue),
        }),
        aster_bin.version().clone(),
    )
}

fn generate_grub_cfg(
    kcmdline: &str,
    skip_grub_menu: bool,
    initramfs_path: Option<String>,
    protocol: &BootProtocol,
) -> String {
    let target_name = get_current_crates().remove(0).name;
    let grub_cfg = include_str!("grub.cfg.template").to_string();

    // Delete the first two lines that notes the file a template file.
    let grub_cfg = grub_cfg.lines().skip(2).collect::<Vec<&str>>().join("\n");
    // Set the timeout style and timeout.
    let grub_cfg = grub_cfg
        .replace(
            "#GRUB_TIMEOUT_STYLE#",
            if skip_grub_menu { "hidden" } else { "menu" },
        )
        .replace("#GRUB_TIMEOUT#", if skip_grub_menu { "0" } else { "5" });
    // Replace all occurrences of "#KERNEL_COMMAND_LINE#" with the desired value.
    let grub_cfg = grub_cfg.replace("#KERNEL_COMMAND_LINE#", kcmdline);
    // Replace the grub commands according to the protocol selected.
    let aster_bin_path_on_device = PathBuf::from("/boot")
        .join(target_name)
        .into_os_string()
        .into_string()
        .unwrap();
    match protocol {
        BootProtocol::Multiboot => grub_cfg
            .replace("#GRUB_CMD_KERNEL#", "multiboot")
            .replace("#KERNEL#", &aster_bin_path_on_device)
            .replace(
                "#GRUB_CMD_INITRAMFS#",
                &if let Some(p) = &initramfs_path {
                    "module --nounzip ".to_owned() + p
                } else {
                    "".to_owned()
                },
            ),
        BootProtocol::Multiboot2 => grub_cfg
            .replace("#GRUB_CMD_KERNEL#", "multiboot2")
            .replace("#KERNEL#", &aster_bin_path_on_device)
            .replace(
                "#GRUB_CMD_INITRAMFS#",
                &if let Some(p) = &initramfs_path {
                    "module2 --nounzip ".to_owned() + p
                } else {
                    "".to_owned()
                },
            ),
        BootProtocol::Linux => grub_cfg
            .replace("#GRUB_CMD_KERNEL#", "linux")
            .replace("#KERNEL#", &aster_bin_path_on_device)
            .replace(
                "#GRUB_CMD_INITRAMFS#",
                &if let Some(p) = &initramfs_path {
                    "initrd ".to_owned() + p
                } else {
                    "".to_owned()
                },
            ),
    }
}

fn get_grub_mkrescue_version(grub_mkrescue: &PathBuf) -> String {
    let mut cmd = new_command_checked_exists(grub_mkrescue);
    cmd.arg("--version");
    let output = cmd.output().unwrap();
    String::from_utf8(output.stdout).unwrap()
}
