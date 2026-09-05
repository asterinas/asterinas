// SPDX-License-Identifier: MPL-2.0

use aster_core::{
    ioctl::{InData, InOutData, NoData, OutData, ioc},
    prelude::*,
};
use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;

use crate::{
    device::{DrmDeviceCapFlags, DrmFeatures},
    file::DrmFile,
};

/// `struct drm_version` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L139>.
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmVersion {
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
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmUnique {
    unique_len: usize,
    unique: usize,
}

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

bitflags::bitflags! {
    /// PRIME capabilities returned for `DRM_CAP_PRIME`.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L683>.
    struct DrmPrimeValue: u64 {
        const IMPORT = 0x1;
        const EXPORT = 0x2;
    }
}

/// `struct drm_get_cap` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L786>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmGetCap {
    capability: u64,
    value: u64,
}

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

/// `struct drm_set_client_cap` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L879>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmSetClientCap {
    capability: u64,
    value: u64,
}

/// `struct drm_auth` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L461>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmAuth {
    magic: u32,
}

pub(crate) type DrmIoctlVersion =
    ioc!(DRM_IOCTL_VERSION, b'd', 0x00, InOutData<DrmVersion>);
pub(crate) type DrmIoctlGetUnique =
    ioc!(DRM_IOCTL_GET_UNIQUE, b'd', 0x01, InOutData<DrmUnique>);
pub(crate) type DrmIoctlGetMagic = ioc!(DRM_IOCTL_GET_MAGIC, b'd', 0x02, OutData<DrmAuth>);
pub(crate) type DrmIoctlGetCap =
    ioc!(DRM_IOCTL_GET_CAP, b'd', 0x0c, InOutData<DrmGetCap>);
pub(crate) type DrmIoctlSetClientCap =
    ioc!(DRM_IOCTL_SET_CLIENT_CAP, b'd', 0x0d, InData<DrmSetClientCap>);
pub(crate) type DrmIoctlAuthMagic = ioc!(DRM_IOCTL_AUTH_MAGIC, b'd', 0x11, InData<DrmAuth>);
pub(crate) type DrmIoctlSetMaster = ioc!(DRM_IOCTL_SET_MASTER, b'd', 0x1e, NoData);
pub(crate) type DrmIoctlDropMaster = ioc!(DRM_IOCTL_DROP_MASTER, b'd', 0x1f, NoData);

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
    pub(crate) fn drm_get_version(&self, cmd: DrmIoctlVersion) -> Result<i32> {
        let mut args: DrmVersion = cmd.read()?;

        let device = self.device();
        let name = device.name();
        let name_len = name.len();
        let desc = device.desc();
        let desc_len = desc.len();

        // These fields are legacy in modern DRM userspace flows.
        // Keep reporting them to preserve `DRM_IOCTL_VERSION` ABI compatibility.
        let date = "0";
        let date_len = date.len();
        let major = 0;
        let minor = 0;
        let patch_level = 0;

        cmd.with_data_ptr(|args_ptr| {
            let userspace = args_ptr.vm();

            // Linux `drm_copy_field` semantics:
            // copy each field independently with truncation,
            // then always report the full source length.
            if args.name_len != 0 {
                let write_len = core::cmp::min(args.name_len, name_len);
                userspace.write_bytes(args.name, &name.as_bytes()[..write_len])?;
            }

            if args.desc_len != 0 {
                let write_len = core::cmp::min(args.desc_len, desc_len);
                userspace.write_bytes(args.desc, &desc.as_bytes()[..write_len])?;
            }

            if args.date_len != 0 {
                let write_len = core::cmp::min(args.date_len, date_len);
                userspace.write_bytes(args.date, &date.as_bytes()[..write_len])?;
            }

            args.name_len = name_len;
            args.desc_len = desc_len;
            args.date_len = date_len;
            args.version_major = major;
            args.version_minor = minor;
            args.version_patchlevel = patch_level;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(crate) fn drm_get_unique(&self, cmd: DrmIoctlGetUnique) -> Result<i32> {
        let mut args: DrmUnique = cmd.read()?;

        // Linux keeps this empty until `DRM_IOCTL_SET_VERSION` has
        // initialized the legacy bus ID for this master context.
        // `SET_VERSION` is not implemented yet, so an empty value is
        // the only compatible result.
        args.unique_len = 0;
        cmd.write(&args)?;
        Ok(0)
    }

    pub(crate) fn drm_get_magic(&self, cmd: DrmIoctlGetMagic) -> Result<i32> {
        let args = DrmAuth {
            magic: self.get_or_allocate_magic()?,
        };
        cmd.write(&args)?;
        Ok(0)
    }

    pub(crate) fn drm_get_cap(&self, cmd: DrmIoctlGetCap) -> Result<i32> {
        use DrmGetCapability::*;

        let mut args: DrmGetCap = cmd.read()?;
        let Ok(cap) = DrmGetCapability::try_from(args.capability) else {
            return_errno_with_message!(Errno::EINVAL, "the DRM device capability is unknown");
        };
        let device = self.device();

        let value = match cap {
            TimestampMonotonic => 1,
            Prime => (DrmPrimeValue::IMPORT | DrmPrimeValue::EXPORT).bits(),
            SyncObj => device.has_feature(DrmFeatures::SYNCOBJ) as u64,
            SyncObjTimeline => device.has_feature(DrmFeatures::SYNCOBJ_TIMELINE) as u64,
            _ => {
                if !device.has_feature(DrmFeatures::MODESET) {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the DRM device lacks modesetting"
                    );
                }
                let device_caps = device.caps();
                let flags = device_caps.flags();
                match cap {
                    DumbBuffer => flags.contains(DrmDeviceCapFlags::DUMB_BUFFER) as u64,
                    VblankHighCrtc => 1,
                    DumbPreferredDepth => device_caps.preferred_color_depth() as u64,
                    DumbPreferShadow => flags.contains(DrmDeviceCapFlags::SHADOW_BUFFER) as u64,
                    AsyncPageFlip => flags.contains(DrmDeviceCapFlags::ASYNC_PAGE_FLIP) as u64,
                    PageFlipTarget => flags.contains(DrmDeviceCapFlags::PAGE_FLIP_TARGET) as u64,
                    CursorWidth => device_caps.cursor_rect().width() as u64,
                    CursorHeight => device_caps.cursor_rect().height() as u64,
                    Addfb2Modifiers => flags.contains(DrmDeviceCapFlags::FB_MODIFIERS) as u64,
                    CrtcInVblankEvent => 1,
                    AtomicAsyncPageFlip => {
                        (device.has_feature(DrmFeatures::ATOMIC)
                            && flags.contains(DrmDeviceCapFlags::ASYNC_PAGE_FLIP))
                            as u64
                    }
                    _ => 0,
                }
            }
        };

        args.value = value;

        cmd.write(&args)?;
        Ok(0)
    }

    pub(crate) fn drm_set_client_cap(&self, cmd: DrmIoctlSetClientCap) -> Result<i32> {
        use DrmSetCapability::*;

        let args: DrmSetClientCap = cmd.read()?;
        let caps = self.caps();
        let device = self.device();

        let Ok(capability) = DrmSetCapability::try_from(args.capability) else {
            return_errno_with_message!(Errno::EINVAL, "the DRM client capability is unknown");
        };

        match capability {
            Stereo3D => caps.set_stereo(parse_boolean_capability(args.value)?),
            UniversalPlane => caps.set_universal_planes(parse_boolean_capability(args.value)?),
            Atomic => {
                if !device.has_feature(DrmFeatures::ATOMIC) {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the DRM device lacks atomic modesetting"
                    );
                }

                match args.value {
                    0..=2 => {
                        let enabled = args.value >= 1;
                        caps.set_atomic(enabled);
                    }
                    _ => return_errno_with_message!(
                        Errno::EINVAL,
                        "the atomic DRM client capability must be zero, one, or two"
                    ),
                }
            }
            AspectRatio => caps.set_aspect_ratio(parse_boolean_capability(args.value)?),
            WritebackConnectors => {
                if !caps.has_atomic() {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the atomic DRM client capability must be enabled before writeback connectors"
                    );
                }

                caps.set_writeback_connectors(parse_boolean_capability(args.value)?);
            }
            CursorPlaneHotspot => {
                if !device.has_feature(DrmFeatures::CURSOR_HOTSPOT) {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the DRM device lacks cursor hotspot support"
                    );
                }

                if !caps.has_atomic() {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the atomic DRM client capability must be enabled before cursor hotspots"
                    );
                }

                caps.set_virtualized_cursor_plane(parse_boolean_capability(args.value)?);
            }
        }
        Ok(0)
    }

    pub(crate) fn drm_auth_magic(&self, cmd: DrmIoctlAuthMagic) -> Result<i32> {
        let args: DrmAuth = cmd.read()?;
        self.authenticate_magic(args.magic)?;
        Ok(0)
    }

    pub(crate) fn drm_set_master(&self, _cmd: DrmIoctlSetMaster) -> Result<i32> {
        self.set_master()?;
        Ok(0)
    }

    pub(crate) fn drm_drop_master(&self, _cmd: DrmIoctlDropMaster) -> Result<i32> {
        self.drop_master()?;
        Ok(0)
    }
}
