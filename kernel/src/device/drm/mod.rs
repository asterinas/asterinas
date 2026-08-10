// SPDX-License-Identifier: MPL-2.0

use aster_drm::{DrmDevice, DrmFeatures};

use crate::{
    device::{
        drm::minor::{DrmMinor, DrmMinorType},
        registry::char,
    },
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::AsPosixThread},
};

mod file;
mod ioctl;
mod minor;

pub(super) fn init_in_first_kthread() -> Result<()> {
    let devices = aster_drm::registered_drm_devices();

    if devices.is_empty() {
        return_errno_with_message!(Errno::ENODEV, "no DRM devices registered");
    }

    let mut any_success = false;

    for (index, device) in devices.iter().enumerate() {
        match register_drm_dev(index as u32, device) {
            Ok(_) => {
                ostd::info!("DRM device {:?} probe correctly!", device.name());
                any_success = true;
            }
            Err(error) => {
                ostd::error!("DrmDevice create error: {:?}", error);
            }
        }
    }

    if any_success {
        Ok(())
    } else {
        return_errno_with_message!(Errno::ENODEV, "all DRM devices register failed");
    }
}

fn register_drm_dev(index: u32, device: &Arc<dyn DrmDevice>) -> Result<()> {
    if device.has_feature(DrmFeatures::COMPUTE_ACCEL) {
        // TODO: Accel node (DRM_ACCEL) is intentionally not implemented for now.
        //
        // Rationale:
        // - The current DRM subsystem only targets primary (cardX) and render (renderDX) nodes.
        // - Modern userspace (Wayland/Mesa/Vulkan) does not rely on accel nodes.
        // - The accel minor is mainly used by specific compute-oriented drivers and is not
        //   required for virtio-gpu or basic KMS/render functionality.
        //
        // let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Accel);
        // char::register(drm_minor)?;
        return_errno!(Errno::EOPNOTSUPP);
    } else {
        if device.has_feature(DrmFeatures::RENDER) {
            let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Render);
            char::register(drm_minor)?;
        }

        // TODO: Control node (controlD*) is intentionally not implemented.
        //
        // Rationale:
        // - The control minor is a legacy DRM node from the pre-KMS / early DRM model.
        // - Modern DRM userspace uses the primary node for display control and KMS, and
        //   uses the render node for rendering.
        // - There is no practical userspace dependency on a separate control node in the
        //   current Wayland/Mesa/virtio-gpu oriented design.
        //
        // let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Control);
        // char::register(drm_minor)?;

        let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Primary);
        char::register(drm_minor)?;
    }
    Ok(())
}

fn has_current_sys_admin() -> bool {
    let binding = current_thread!();
    let Some(posix_thread) = binding.as_posix_thread() else {
        return false;
    };
    posix_thread
        .credentials()
        .effective_capset()
        .contains(CapSet::SYS_ADMIN)
}
