// SPDX-License-Identifier: MPL-2.0

mod gem;
mod general;
mod kms;

use aster_core::{dispatch_ioctl, prelude::*, util::ioctl::RawIoctl};

use crate::{file::DrmFile, has_current_sys_admin, minor::DrmMinorType};

impl DrmFile {
    pub(super) fn dispatch_ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            // General ioctl cmds.
            cmd @ Version => {
                self.check_ioctl_access(DrmIoctlAccess::RENDER_ALLOW)?;
                self.drm_get_version(cmd)
            }
            cmd @ GetUnique => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_get_unique(cmd)
            }
            cmd @ GetMagic => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_get_magic(cmd)
            }
            cmd @ GetCap => {
                self.check_ioctl_access(DrmIoctlAccess::RENDER_ALLOW)?;
                self.drm_get_cap(cmd)
            }
            cmd @ AuthMagic => {
                self.check_ioctl_access(DrmIoctlAccess::MASTER)?;
                self.drm_auth_magic(cmd)
            }
            cmd @ SetMaster => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_set_master(cmd)
            }
            cmd @ DropMaster => {
                self.check_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_drop_master(cmd)
            }
            // GEM ioctl cmds.
            cmd @ GemClose => {
                self.check_gem_ioctl_access(DrmIoctlAccess::RENDER_ALLOW)?;
                self.drm_gem_close(cmd)
            }
            cmd @ ModeCreateDumb => {
                self.check_gem_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_create_dumb(cmd)
            }
            cmd @ ModeMapDumb => {
                self.check_gem_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_map_dumb(cmd)
            }
            cmd @ ModeDestroyDumb => {
                self.check_gem_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_destroy_dumb(cmd)
            }
            // KMS ioctl cmds.
            cmd @ SetClientCap => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_set_client_cap(cmd)
            }
            cmd @ ModeGetResources => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_get_resources(cmd)
            }
            cmd @ ModeGetCrtc => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_get_crtc(cmd)
            }
            cmd @ ModeSetCrtc => {
                self.check_kms_ioctl_access(DrmIoctlAccess::MASTER)?;
                self.drm_mode_set_crtc(cmd)
            }
            cmd @ ModeGetEncoder => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_get_encoder(cmd)
            }
            cmd @ ModeGetConnector => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_get_connector(cmd)
            }
            cmd @ ModeGetProperty => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_get_property(cmd)
            }
            cmd @ ModeGetPropBlob => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_get_blob(cmd)
            }
            cmd @ ModeAddFb => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_add_fb(cmd)
            }
            cmd @ ModeRmFb => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_rm_fb(cmd)
            }
            cmd @ ModeDirtyFb => {
                self.check_kms_ioctl_access(DrmIoctlAccess::MASTER)?;
                self.drm_mode_dirty_fb(cmd)
            }
            cmd @ ModeGetPlaneResources => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_get_plane_resources(cmd)
            }
            cmd @ ModeGetPlane => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_get_plane(cmd)
            }
            cmd @ ModeObjectGetProps => {
                self.check_kms_ioctl_access(DrmIoctlAccess::empty())?;
                self.drm_mode_object_get_props(cmd)
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

    fn check_kms_ioctl_access(&self, required_access: DrmIoctlAccess) -> Result<()> {
        if self.device().as_kms_ops().is_none() {
            return_errno_with_message!(Errno::EOPNOTSUPP, "the DRM device does not support KMS");
        }

        self.check_ioctl_access(required_access)
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

    use crate::ioctl::{
        gem::{DrmGemClose, DrmModeCreateDumb, DrmModeDestroyDumb, DrmModeMapDumb},
        general::{DrmAuth, DrmGetCap, DrmUnique, DrmVersion},
        kms::{
            DrmModeCrtc, DrmModeFbCmd, DrmModeFbDirtyCmd, DrmModeGetBlob, DrmModeGetConnector,
            DrmModeGetEncoder, DrmModeGetPlane, DrmModeGetPlaneRes, DrmModeGetProperty,
            DrmModeGetResources, DrmModeObjectGetProps, DrmSetClientCap,
        },
    };

    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h>
    pub(super) type Version                 = ioc!(DRM_IOCTL_VERSION,                   b'd', 0x00, InOutData<DrmVersion>);
    pub(super) type GetUnique               = ioc!(DRM_IOCTL_GET_UNIQUE,                b'd', 0x01, InOutData<DrmUnique>);
    pub(super) type GetMagic                = ioc!(DRM_IOCTL_GET_MAGIC,                 b'd', 0x02, OutData<DrmAuth>);
    pub(super) type GemClose                = ioc!(DRM_IOCTL_GEM_CLOSE,                 b'd', 0x09, InData<DrmGemClose>);
    pub(super) type GetCap                  = ioc!(DRM_IOCTL_GET_CAP,                   b'd', 0x0c, InOutData<DrmGetCap>);
    pub(super) type SetClientCap            = ioc!(DRM_IOCTL_SET_CLIENT_CAP,            b'd', 0x0d, InData<DrmSetClientCap>);
    pub(super) type AuthMagic               = ioc!(DRM_IOCTL_AUTH_MAGIC,                b'd', 0x11, InData<DrmAuth>);
    pub(super) type SetMaster               = ioc!(DRM_IOCTL_SET_MASTER,                b'd', 0x1e, NoData);
    pub(super) type DropMaster              = ioc!(DRM_IOCTL_DROP_MASTER,               b'd', 0x1f, NoData);
    pub(super) type ModeCreateDumb          = ioc!(DRM_IOCTL_MODE_CREATE_DUMB,          b'd', 0xb2, InOutData<DrmModeCreateDumb>);
    pub(super) type ModeMapDumb             = ioc!(DRM_IOCTL_MODE_MAP_DUMB,             b'd', 0xb3, InOutData<DrmModeMapDumb>);
    pub(super) type ModeDestroyDumb         = ioc!(DRM_IOCTL_MODE_DESTROY_DUMB,         b'd', 0xb4, InOutData<DrmModeDestroyDumb>);
    pub(super) type ModeGetResources        = ioc!(DRM_IOCTL_MODE_GETRESOURCES,         b'd', 0xa0, InOutData<DrmModeGetResources>);
    pub(super) type ModeGetCrtc             = ioc!(DRM_IOCTL_MODE_GETCRTC,              b'd', 0xa1, InOutData<DrmModeCrtc>);
    pub(super) type ModeSetCrtc             = ioc!(DRM_IOCTL_MODE_SETCRTC,              b'd', 0xa2, InOutData<DrmModeCrtc>);
    pub(super) type ModeGetEncoder          = ioc!(DRM_IOCTL_MODE_GETENCODER,           b'd', 0xa6, InOutData<DrmModeGetEncoder>);
    pub(super) type ModeGetConnector        = ioc!(DRM_IOCTL_MODE_GETCONNECTOR,         b'd', 0xa7, InOutData<DrmModeGetConnector>);
    pub(super) type ModeGetProperty         = ioc!(DRM_IOCTL_MODE_GETPROPERTY,          b'd', 0xaa, InOutData<DrmModeGetProperty>);
    pub(super) type ModeGetPropBlob         = ioc!(DRM_IOCTL_MODE_GETPROPBLOB,          b'd', 0xac, InOutData<DrmModeGetBlob>);
    pub(super) type ModeAddFb               = ioc!(DRM_IOCTL_MODE_ADDFB,                b'd', 0xae, InOutData<DrmModeFbCmd>);
    pub(super) type ModeRmFb                = ioc!(DRM_IOCTL_MODE_RMFB,                 b'd', 0xaf, InOutData<u32>);
    pub(super) type ModeDirtyFb             = ioc!(DRM_IOCTL_MODE_DIRTYFB,              b'd', 0xb1, InOutData<DrmModeFbDirtyCmd>);
    pub(super) type ModeGetPlaneResources   = ioc!(DRM_IOCTL_MODE_GETPLANERESOURCES,    b'd', 0xb5, InOutData<DrmModeGetPlaneRes>);
    pub(super) type ModeGetPlane            = ioc!(DRM_IOCTL_MODE_GETPLANE,             b'd', 0xb6, InOutData<DrmModeGetPlane>);
    pub(super) type ModeObjectGetProps      = ioc!(DRM_IOCTL_MODE_OBJ_GETPROPERTIES,    b'd', 0xb9, InOutData<DrmModeObjectGetProps>);
}
