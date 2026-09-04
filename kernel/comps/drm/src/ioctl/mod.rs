// SPDX-License-Identifier: MPL-2.0

use aster_core::{
    ioctl::{RawIoctl, dispatch_ioctl},
    prelude::*,
};

use crate::has_current_sys_admin;

pub(crate) mod general;
use general::*;

bitflags::bitflags! {
    pub(crate) struct DrmIoctlFlags: u32 {
        const DEFAULT          = 0;
        const AUTH         = 1 << 0; // authenticated or master
        const MASTER       = 1 << 1; // requires current file is master
        const ROOT_ONLY    = 1 << 2; // requires CAP_SYS_ADMIN
        const RENDER_ALLOW = 1 << 3; // allowed on render node
        const MODESET      = 1 << 4; // requires a modeset-capable DRM driver
        const ATOMIC       = 1 << 5; // requires driver atomic modesetting support
        const GEM          = 1 << 6; // requires GEM memory-management support
        const SYNCOBJ      = 1 << 7; // requires sync object support
        const SYNCOBJ_TIMELINE = 1 << 8; // requires timeline sync object support
        const CURSOR_HOTSPOT   = 1 << 9; // requires virtualized cursor hotspot support
    }
}

impl DrmFile {
    fn check_required_driver_features(&self, required_flags: DrmIoctlFlags) -> Result<()> {
        let required_features = [
            (DrmIoctlFlags::MODESET, DrmFeatures::MODESET),
            (DrmIoctlFlags::ATOMIC, DrmFeatures::ATOMIC),
            (DrmIoctlFlags::GEM, DrmFeatures::GEM),
            (DrmIoctlFlags::SYNCOBJ, DrmFeatures::SYNCOBJ),
            (
                DrmIoctlFlags::SYNCOBJ_TIMELINE,
                DrmFeatures::SYNCOBJ_TIMELINE,
            ),
            (DrmIoctlFlags::CURSOR_HOTSPOT, DrmFeatures::CURSOR_HOTSPOT),
        ];

        for (required_flag, driver_feature) in required_features {
            if required_flags.contains(required_flag) && !self.has_feature(driver_feature) {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "the DRM device lacks a feature required by the ioctl"
                );
            }
        }

        Ok(())
    }

    fn check_ioctl_flags(&self, required_flags: DrmIoctlFlags) -> Result<()> {
        match self.minor_type() {
            DrmMinorType::Primary => {
                if required_flags.contains(DrmIoctlFlags::AUTH) && !self.is_authenticated() {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "the DRM ioctl requires an authenticated primary client"
                    );
                }
            }
            DrmMinorType::Render => {
                if !required_flags.contains(DrmIoctlFlags::RENDER_ALLOW) {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "the DRM ioctl is not allowed on a render node"
                    );
                }
            }
            _ => {
                // TODO: control/accel policy
                return_errno_with_message!(
                    Errno::EACCES,
                    "the DRM ioctl is not supported on this minor type"
                );
            }
        }

        if required_flags.contains(DrmIoctlFlags::ROOT_ONLY) && !has_current_sys_admin() {
            return_errno_with_message!(Errno::EACCES, "the DRM ioctl requires CAP_SYS_ADMIN");
        }
        if required_flags.contains(DrmIoctlFlags::MASTER) && !self.is_current_master() {
            return_errno_with_message!(Errno::EACCES, "the DRM client is not the current master");
        }

        self.check_required_driver_features(required_flags)
    }

    pub(crate) fn dispatch_ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            // General ioctl cmds.
            cmd @ DrmIoctlVersion => {
                self.check_ioctl_flags(DrmIoctlFlags::RENDER_ALLOW)?;
                self.drm_get_version(cmd)
            }
            cmd @ DrmIoctlGetUnique => {
                self.check_ioctl_flags(DrmIoctlFlags::DEFAULT)?;
                self.drm_get_unique(cmd)
            }
            cmd @ DrmIoctlGetMagic => {
                self.check_ioctl_flags(DrmIoctlFlags::DEFAULT)?;
                self.drm_get_magic(cmd)
            }
            cmd @ DrmIoctlGetCap => {
                self.check_ioctl_flags(DrmIoctlFlags::RENDER_ALLOW)?;
                self.drm_get_cap(cmd)
            }
            cmd @ DrmIoctlSetClientCap => {
                self.check_ioctl_flags(DrmIoctlFlags::MODESET)?;
                self.drm_set_client_cap(cmd)
            }
            cmd @ DrmIoctlAuthMagic => {
                self.check_ioctl_flags(DrmIoctlFlags::MASTER)?;
                self.drm_auth_magic(cmd)
            }
            cmd @ DrmIoctlSetMaster => {
                self.check_ioctl_flags(DrmIoctlFlags::DEFAULT)?;
                self.drm_set_master(cmd)
            }
            cmd @ DrmIoctlDropMaster => {
                self.check_ioctl_flags(DrmIoctlFlags::DEFAULT)?;
                self.drm_drop_master(cmd)
            }
            _ => match self.device().handle_command(self, raw_ioctl) {
                Some(result) => result,
                None => {
                    ostd::warn!(
                        "drm: unknown ioctl minor={:?} cmd={:#x}",
                        self.minor_type(),
                        raw_ioctl.cmd()
                    );
                    return_errno_with_message!(Errno::ENOTTY, "the DRM ioctl command is unknown")
                }
            },
        })
    }
}

use crate::{device::DrmFeatures, file::DrmFile, minor::DrmMinorType};
