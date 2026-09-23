// SPDX-License-Identifier: MPL-2.0

use aster_core::prelude::*;

use crate::{
    file::DrmFile,
    ioctl::{
        DrmIoctlGemClose, DrmIoctlModeCreateDumb, DrmIoctlModeDestroyDumb, DrmIoctlModeMapDumb,
    },
};

impl DrmFile {
    pub(super) fn drm_gem_close(&self, cmd: DrmIoctlGemClose) -> Result<i32> {
        let args = cmd.read()?;
        self.remove_gem_object(args.handle)?;

        Ok(0)
    }

    pub(super) fn drm_mode_create_dumb(&self, cmd: DrmIoctlModeCreateDumb) -> Result<i32> {
        let mut args = cmd.read()?;

        if args.width == 0 || args.height == 0 || args.bpp == 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "the dumb-buffer dimensions and bits per pixel must be nonzero"
            );
        }

        let bits_per_row = u64::from(args.width) * u64::from(args.bpp);
        let Ok(pitch) = u32::try_from(bits_per_row.div_ceil(8)) else {
            return_errno_with_message!(Errno::EINVAL, "the dumb-buffer pitch overflows u32");
        };
        let Some(size) = (pitch as usize).checked_mul(args.height as usize) else {
            return_errno_with_message!(Errno::EINVAL, "the dumb-buffer size overflows usize");
        };

        let gem_ops = self.device().as_gem_ops().ok_or(Errno::EOPNOTSUPP)?;
        let gem_object = gem_ops.create_dumb(size)?;

        args.pitch = pitch;
        args.size = gem_object.size() as u64;

        args.handle = self.add_gem_object(gem_object)?;
        if let Err(err) = cmd.write(&args) {
            let _ = self.remove_gem_object(args.handle);
            return Err(err);
        }

        Ok(0)
    }

    pub(super) fn drm_mode_map_dumb(&self, cmd: DrmIoctlModeMapDumb) -> Result<i32> {
        let mut args = cmd.read()?;
        args.offset = self.map_gem_handle(args.handle)?;
        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn drm_mode_destroy_dumb(&self, cmd: DrmIoctlModeDestroyDumb) -> Result<i32> {
        let args = cmd.read()?;
        self.remove_gem_object(args.handle)?;

        Ok(0)
    }
}

/// `struct drm_gem_close` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L600-L605>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmGemClose {
    handle: u32,
    pad: u32,
}

/// `struct drm_mode_create_dumb` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L1057-L1079>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeCreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

/// `struct drm_mode_map_dumb` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L1081-L1092>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeMapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

/// `struct drm_mode_destroy_dumb` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L1094-L1096>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeDestroyDumb {
    handle: u32,
}
