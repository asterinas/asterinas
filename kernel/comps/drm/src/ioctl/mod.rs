// SPDX-License-Identifier: MPL-2.0

mod gem;
mod general;

use aster_core::{dispatch_ioctl, prelude::*, util::ioctl::RawIoctl};
use ioctl_defs::*;

use crate::{device::DrmFeatures, file::DrmFile, has_current_sys_admin, minor::DrmMinorType};

impl DrmFile {
    pub(super) fn dispatch_ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            // General ioctl cmds.
            cmd @ DrmIoctlVersion => {
                self.check_ioctl_access(DrmIoctlAccess::RENDER_ALLOW)?;
                self.drm_get_version(cmd)
            }
            cmd @ DrmIoctlGetUnique => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_get_unique(cmd)
            }
            cmd @ DrmIoctlGetMagic => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_get_magic(cmd)
            }
            cmd @ DrmIoctlGetCap => {
                self.check_ioctl_access(DrmIoctlAccess::RENDER_ALLOW)?;
                self.drm_get_cap(cmd)
            }
            cmd @ DrmIoctlSetClientCap => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_set_client_cap(cmd)
            }
            cmd @ DrmIoctlAuthMagic => {
                self.check_ioctl_access(DrmIoctlAccess::MASTER)?;
                self.drm_auth_magic(cmd)
            }
            cmd @ DrmIoctlSetMaster => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_set_master(cmd)
            }
            cmd @ DrmIoctlDropMaster => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_drop_master(cmd)
            }
            // GEM ioctl cmds.
            cmd @ DrmIoctlGemClose => {
                self.check_gem_ioctl_access(DrmIoctlAccess::RENDER_ALLOW)?;
                self.drm_gem_close(cmd)
            }
            cmd @ DrmIoctlModeCreateDumb => {
                self.check_gem_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_create_dumb(cmd)
            }
            cmd @ DrmIoctlModeMapDumb => {
                self.check_gem_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_map_dumb(cmd)
            }
            cmd @ DrmIoctlModeDestroyDumb => {
                self.check_gem_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_destroy_dumb(cmd)
            }
            _ => {
                ostd::warn!(
                    "unknown ioctl minor={:?} cmd={:#x}",
                    self.minor_type(),
                    raw_ioctl.cmd()
                );
                return_errno_with_message!(Errno::ENOTTY, "the DRM ioctl command is unknown")
            }
        })
    }

    fn check_gem_ioctl_access(&self, required_access: DrmIoctlAccess) -> Result<()> {
        if self.device().as_gem_ops().is_none() {
            return_errno_with_message!(Errno::EOPNOTSUPP, "the DRM device does not support GEM");
        }

        self.check_ioctl_access(required_access)
    }

    fn check_ioctl_features(&self, required_features: DrmFeatures) -> Result<()> {
        if !self.has_features(required_features) {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "the DRM device lacks a feature required by the ioctl"
            );
        }

        Ok(())
    }

    fn check_ioctl_access(&self, required_access: DrmIoctlAccess) -> Result<()> {
        match self.minor_type() {
            DrmMinorType::Primary => {
                if required_access.contains(DrmIoctlAccess::AUTH) && !self.is_authenticated() {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "the DRM ioctl requires an authenticated primary client"
                    );
                }
            }
            DrmMinorType::Render => {
                if !required_access.contains(DrmIoctlAccess::RENDER_ALLOW) {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "the DRM ioctl is not allowed on a render node"
                    );
                }
            }
        }

        if required_access.contains(DrmIoctlAccess::ROOT_ONLY) && !has_current_sys_admin() {
            return_errno_with_message!(Errno::EACCES, "the DRM ioctl requires CAP_SYS_ADMIN");
        }
        if required_access.contains(DrmIoctlAccess::MASTER) && !self.is_master() {
            return_errno_with_message!(Errno::EACCES, "the DRM client is not the current master");
        }

        Ok(())
    }
}

bitflags::bitflags! {
    struct DrmIoctlAccess: u32 {
        /// Requires the client to be authenticated or the current master.
        const AUTH = 1 << 0;
        /// Requires the current file to be the DRM master.
        const MASTER = 1 << 1;
        /// Requires the caller to have `CAP_SYS_ADMIN`.
        const ROOT_ONLY = 1 << 2;
        /// Allows the ioctl on a render node.
        const RENDER_ALLOW = 1 << 3;
    }
}

mod ioctl_defs {
    use aster_core::{
        ioc,
        util::ioctl::{InData, InOutData, NoData, OutData},
    };

    use super::general::{DrmAuth, DrmGetCap, DrmSetClientCap, DrmUnique, DrmVersion};
    use crate::ioctl::gem::{DrmGemClose, DrmModeCreateDumb, DrmModeDestroyDumb, DrmModeMapDumb};

    pub(super) type DrmIoctlVersion =
        ioc!(DRM_IOCTL_VERSION, b'd', 0x00, InOutData<DrmVersion>);
    pub(super) type DrmIoctlGetUnique =
        ioc!(DRM_IOCTL_GET_UNIQUE, b'd', 0x01, InOutData<DrmUnique>);
    pub(super) type DrmIoctlGetMagic =
        ioc!(DRM_IOCTL_GET_MAGIC, b'd', 0x02, OutData<DrmAuth>);
    pub(super) type DrmIoctlGemClose = ioc!(
        DRM_IOCTL_GEM_CLOSE,
        b'd',
        0x09,
        InData<DrmGemClose>
    );
    pub(super) type DrmIoctlGetCap =
        ioc!(DRM_IOCTL_GET_CAP, b'd', 0x0c, InOutData<DrmGetCap>);
    pub(super) type DrmIoctlSetClientCap =
        ioc!(DRM_IOCTL_SET_CLIENT_CAP, b'd', 0x0d, InData<DrmSetClientCap>);
    pub(super) type DrmIoctlAuthMagic =
        ioc!(DRM_IOCTL_AUTH_MAGIC, b'd', 0x11, InData<DrmAuth>);
    pub(super) type DrmIoctlSetMaster = ioc!(DRM_IOCTL_SET_MASTER, b'd', 0x1e, NoData);
    pub(super) type DrmIoctlDropMaster = ioc!(DRM_IOCTL_DROP_MASTER, b'd', 0x1f, NoData);
    pub(super) type DrmIoctlModeCreateDumb = ioc!(
        DRM_IOCTL_MODE_CREATE_DUMB,
        b'd',
        0xb2,
        InOutData<DrmModeCreateDumb>
    );
    pub(super) type DrmIoctlModeMapDumb = ioc!(
        DRM_IOCTL_MODE_MAP_DUMB,
        b'd',
        0xb3,
        InOutData<DrmModeMapDumb>
    );
    pub(super) type DrmIoctlModeDestroyDumb = ioc!(
        DRM_IOCTL_MODE_DESTROY_DUMB,
        b'd',
        0xb4,
        InOutData<DrmModeDestroyDumb>
    );
}
