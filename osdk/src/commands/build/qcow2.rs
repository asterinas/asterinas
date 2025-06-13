// SPDX-License-Identifier: MPL-2.0

use std::process;

use crate::{
    bundle::{
        file::BundleFile,
        vm_image::{AsterQcow2ImageMeta, AsterVmImage, AsterVmImageType},
    },
    error_msg,
    util::new_command_checked_exists,
};

pub fn convert_iso_to_qcow2(iso: AsterVmImage) -> AsterVmImage {
    let AsterVmImageType::GrubIso(meta) = iso.typ() else {
        panic!("Expected a GRUB ISO image, but got: {:?}", iso.typ());
    };
    let iso_path = iso.path();
    let qcow2_path = iso_path.with_extension("qcow2");
    // Convert the ISO to QCOW2 using `qemu-img`.
    let mut qemu_img = new_command_checked_exists("qemu-img");
    qemu_img.args([
        "convert",
        "-O",
        "qcow2",
        iso_path.to_str().unwrap(),
        qcow2_path.to_str().unwrap(),
    ]);
    info!("Converting the ISO to QCOW2 using {:#?}", qemu_img);
    if !qemu_img.status().unwrap().success() {
        error_msg!("Failed to convert the ISO to QCOW2: {:?}", qemu_img);
        process::exit(1);
    }
    AsterVmImage::new(
        qcow2_path,
        AsterVmImageType::Qcow2(AsterQcow2ImageMeta {
            grub_version: meta.grub_version.clone(),
        }),
        iso.aster_version().clone(),
    )
}
