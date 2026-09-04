// SPDX-License-Identifier: MPL-2.0

use aster_core::prelude::*;
use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;

use super::ioctl_defs::*;
use crate::{
    device::DrmFeatures,
    file::{DrmClientCaps, DrmFile},
};

const DRM_DEFAULT_CURSOR_WIDTH: u64 = 64;
const DRM_DEFAULT_CURSOR_HEIGHT: u64 = 64;

/// `struct drm_version` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L139>.
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmVersion {
    version_major: i32,
    version_minor: i32,
    version_patchlevel: i32,

    name_len: usize,
    name: usize,
    date_len: usize,
    date: usize,
    desc_len: usize,
    desc: usize,
}

/// `struct drm_unique` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L156>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmUnique {
    unique_len: usize,
    unique: usize,
}

/// `struct drm_get_cap` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L786>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmGetCap {
    capability: u64,
    value: u64,
}

/// `struct drm_set_client_cap` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L879>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmSetClientCap {
    capability: u64,
    value: u64,
}

/// `struct drm_auth` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L461>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmAuth {
    magic: u32,
}

fn parse_boolean_capability(value: u64) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => return_errno_with_message!(
            Errno::EINVAL,
            "a boolean DRM client capability must be zero or one"
        ),
    }
}

impl DrmFile {
    pub(super) fn drm_get_version(&self, cmd: DrmIoctlVersion) -> Result<i32> {
        let device = self.device();
        let name = device.name();
        let desc = device.desc();

        // These fields are legacy in modern DRM userspace flows.
        // They are still reported to preserve `DRM_IOCTL_VERSION` ABI compatibility.
        let date = "0";
        let major = 0;
        let minor = 0;
        let patch_level = 0;

        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmVersion = args_ptr.read()?;
            let userspace = args_ptr.vm();

            args.version_major = major;
            args.version_minor = minor;
            args.version_patchlevel = patch_level;

            // Linux copies the ioctl argument back even if copying one of the
            // referenced string fields fails.
            let copy_result: Result<()> = (|| {
                copy_drm_field(&userspace, args.name, &mut args.name_len, name.as_bytes())?;
                copy_drm_field(&userspace, args.date, &mut args.date_len, date.as_bytes())?;
                copy_drm_field(&userspace, args.desc, &mut args.desc_len, desc.as_bytes())?;
                Ok(())
            })();

            args_ptr.write(&args)?;
            copy_result
        })?;

        Ok(0)
    }

    pub(super) fn drm_get_unique(&self, cmd: DrmIoctlGetUnique) -> Result<i32> {
        let mut args: DrmUnique = cmd.read()?;

        // Linux keeps this empty until `DRM_IOCTL_SET_VERSION` has
        // initialized the legacy bus ID for this master context.
        // `SET_VERSION` is not implemented yet, so an empty value is
        // the only compatible result.
        args.unique_len = 0;
        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_get_magic(&self, cmd: DrmIoctlGetMagic) -> Result<i32> {
        let args = DrmAuth {
            magic: self.get_or_allocate_magic()?,
        };
        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_get_cap(&self, cmd: DrmIoctlGetCap) -> Result<i32> {
        /// DRM device capabilities accepted by `DRM_IOCTL_GET_CAP`.
        ///
        /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L628>.
        #[repr(u64)]
        #[derive(Debug, TryFromInt)]
        enum DrmGetCapability {
            DumbBuffer = 0x1,
            VblankHighCrtc = 0x2,
            DumbPreferredDepth = 0x3,
            DumbPreferShadow = 0x4,
            Prime = 0x5,
            TimestampMonotonic = 0x6,
            AsyncPageFlip = 0x7,
            CursorWidth = 0x8,
            CursorHeight = 0x9,
            Addfb2Modifiers = 0x10,
            PageFlipTarget = 0x11,
            CrtcInVblankEvent = 0x12,
            SyncObj = 0x13,
            SyncObjTimeline = 0x14,
            AtomicAsyncPageFlip = 0x15,
        }

        let mut args: DrmGetCap = cmd.read()?;
        let Ok(cap) = DrmGetCapability::try_from(args.capability) else {
            return_errno_with_message!(Errno::EINVAL, "the DRM device capability is unknown");
        };
        let device = self.device();

        let value = match cap {
            DrmGetCapability::TimestampMonotonic => 1,
            DrmGetCapability::Prime => {
                // TODO: Report PRIME import and export support after
                // `DRM_IOCTL_PRIME_FD_TO_HANDLE` and `DRM_IOCTL_PRIME_HANDLE_TO_FD`
                // are implemented.
                0
            }
            DrmGetCapability::SyncObj => device.has_features(DrmFeatures::SYNCOBJ) as u64,
            DrmGetCapability::SyncObjTimeline => {
                device.has_features(DrmFeatures::SYNCOBJ_TIMELINE) as u64
            }
            _ => {
                if !device.has_features(DrmFeatures::MODESET) {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the DRM device lacks modesetting"
                    );
                }

                match cap {
                    DrmGetCapability::DumbBuffer => {
                        // TODO: Derive this capability from the optional dumb-buffer
                        // operation once that interface is introduced.
                        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/gpu/drm/drm_ioctl.c#L262-L265>.
                        0
                    }
                    DrmGetCapability::VblankHighCrtc => 1,
                    // TODO: Once KMS is integrated, obtain the mode config from the
                    // registered DRM device and check that it exists before reporting
                    // mode-config-dependent capabilities below.
                    DrmGetCapability::DumbPreferredDepth
                    | DrmGetCapability::DumbPreferShadow
                    | DrmGetCapability::AsyncPageFlip
                    | DrmGetCapability::Addfb2Modifiers
                    | DrmGetCapability::AtomicAsyncPageFlip => 0,
                    DrmGetCapability::CursorWidth => DRM_DEFAULT_CURSOR_WIDTH,
                    DrmGetCapability::CursorHeight => DRM_DEFAULT_CURSOR_HEIGHT,
                    DrmGetCapability::PageFlipTarget => {
                        // TODO: Derive this capability from the CRTC operations once
                        // the KMS interface is introduced.
                        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/gpu/drm/drm_ioctl.c#L278-L283>.
                        0
                    }
                    DrmGetCapability::CrtcInVblankEvent => 1,
                    _ => 0,
                }
            }
        };

        args.value = value;

        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_set_client_cap(&self, cmd: DrmIoctlSetClientCap) -> Result<i32> {
        /// DRM client capabilities accepted by `DRM_IOCTL_SET_CLIENT_CAP`.
        ///
        /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L791>.
        #[repr(u64)]
        #[derive(Debug, TryFromInt)]
        enum DrmSetCapability {
            Stereo3D = 0x1,
            UniversalPlane = 0x2,
            Atomic = 0x3,
            AspectRatio = 0x4,
            WritebackConnectors = 0x5,
            CursorPlaneHotspot = 0x6,
        }

        let args: DrmSetClientCap = cmd.read()?;
        let device = self.device();

        let Ok(cap) = DrmSetCapability::try_from(args.capability) else {
            return_errno_with_message!(Errno::EINVAL, "the DRM client capability is unknown");
        };

        match cap {
            DrmSetCapability::Stereo3D => self.set_client_caps(
                DrmClientCaps::STEREO_3D,
                parse_boolean_capability(args.value)?,
            ),
            DrmSetCapability::UniversalPlane => self.set_client_caps(
                DrmClientCaps::UNIVERSAL_PLANES,
                parse_boolean_capability(args.value)?,
            ),
            DrmSetCapability::Atomic => {
                if !device.has_features(DrmFeatures::ATOMIC) {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the DRM device lacks atomic modesetting"
                    );
                }

                match args.value {
                    0..=2 => {
                        let enabled = args.value >= 1;
                        self.set_client_caps(
                            DrmClientCaps::ATOMIC
                                | DrmClientCaps::UNIVERSAL_PLANES
                                | DrmClientCaps::ASPECT_RATIO,
                            enabled,
                        );
                    }
                    _ => return_errno_with_message!(
                        Errno::EINVAL,
                        "the atomic DRM client capability must be zero, one, or two"
                    ),
                }
            }
            DrmSetCapability::AspectRatio => self.set_client_caps(
                DrmClientCaps::ASPECT_RATIO,
                parse_boolean_capability(args.value)?,
            ),
            DrmSetCapability::WritebackConnectors => {
                if !self.has_client_caps(DrmClientCaps::ATOMIC) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the atomic DRM client capability must be enabled before writeback connectors"
                    );
                }

                self.set_client_caps(
                    DrmClientCaps::WRITEBACK_CONNECTORS,
                    parse_boolean_capability(args.value)?,
                );
            }
            DrmSetCapability::CursorPlaneHotspot => {
                if !device.has_features(DrmFeatures::CURSOR_HOTSPOT) {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the DRM device lacks cursor hotspot support"
                    );
                }

                if !self.has_client_caps(DrmClientCaps::ATOMIC) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the atomic DRM client capability must be enabled before cursor hotspots"
                    );
                }

                self.set_client_caps(
                    DrmClientCaps::CURSOR_PLANE_HOTSPOT,
                    parse_boolean_capability(args.value)?,
                );
            }
        }
        Ok(0)
    }

    pub(super) fn drm_auth_magic(&self, cmd: DrmIoctlAuthMagic) -> Result<i32> {
        let args: DrmAuth = cmd.read()?;
        self.authenticate_magic(args.magic)?;
        Ok(0)
    }

    pub(super) fn drm_set_master(&self, _cmd: DrmIoctlSetMaster) -> Result<i32> {
        self.set_master()?;
        Ok(0)
    }

    pub(super) fn drm_drop_master(&self, _cmd: DrmIoctlDropMaster) -> Result<i32> {
        self.drop_master()?;
        Ok(0)
    }
}

fn copy_drm_field(
    userspace: &impl VmIo,
    user_addr: usize,
    user_capacity: &mut usize,
    value: &[u8],
) -> Result<()> {
    let copy_len = core::cmp::min(*user_capacity, value.len());
    *user_capacity = value.len();

    if user_addr != 0 && copy_len != 0 {
        userspace.write_bytes(user_addr, &value[..copy_len])?;
    }

    Ok(())
}
