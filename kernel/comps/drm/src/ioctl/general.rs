// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_core::prelude::*;
use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;

use crate::{
    device::DrmFeatures,
    file::DrmFile,
    ioctl::ioctl_defs,
    kms::{
        objects::KmsObjectIndex,
        vblank::{DrmVblankEvent, DrmVblankEventData, DrmVblankWaitResult, MAX_PASSED_DISTANCE},
    },
};

impl DrmFile {
    pub(super) fn drm_get_version(&self, cmd: ioctl_defs::Version) -> Result<i32> {
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

    pub(super) fn drm_get_unique(&self, cmd: ioctl_defs::GetUnique) -> Result<i32> {
        let mut args: DrmUnique = cmd.read()?;

        // Linux keeps this empty until `DRM_IOCTL_SET_VERSION` has
        // initialized the legacy bus ID for this master context.
        // `SET_VERSION` is not implemented yet, so an empty value is
        // the only compatible result.
        args.unique_len = 0;
        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_get_magic(&self, cmd: ioctl_defs::GetMagic) -> Result<i32> {
        let args = DrmAuth {
            magic: self.get_or_allocate_magic()?,
        };
        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_get_cap(&self, cmd: ioctl_defs::GetCap) -> Result<i32> {
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
            _ => self.kms_get_cap(cap)?,
        };

        args.value = value;

        cmd.write(&args)?;
        Ok(0)
    }

    fn kms_get_cap(&self, cap: DrmGetCapability) -> Result<u64> {
        let device = self.device();
        let kms_ops = device.as_kms_ops().ok_or_else(|| {
            Error::with_message(Errno::EOPNOTSUPP, "the DRM device lacks modesetting")
        })?;
        let mode_config = kms_ops.mode_config();

        let result = match cap {
            DrmGetCapability::DumbBuffer => {
                // `DrmGemOps` currently requires `create_dumb`,
                // so checking `as_gem_ops()` is sufficient to report dumb-buffer support.
                device.as_gem_ops().is_some() as u64
            }
            DrmGetCapability::VblankHighCrtc => 1,
            DrmGetCapability::DumbPreferredDepth => {
                u64::from(mode_config.preferred_dumb_buffer_depth())
            }
            DrmGetCapability::DumbPreferShadow => mode_config.prefer_shadow_buffer() as u64,
            DrmGetCapability::AsyncPageFlip => mode_config.supports_async_page_flip() as u64,
            DrmGetCapability::CursorWidth => {
                const DRM_DEFAULT_CURSOR_WIDTH: u64 = 64;
                mode_config
                    .cursor_size()
                    .map_or(DRM_DEFAULT_CURSOR_WIDTH, |size| u64::from(size.width()))
            }
            DrmGetCapability::CursorHeight => {
                const DRM_DEFAULT_CURSOR_HEIGHT: u64 = 64;
                mode_config
                    .cursor_size()
                    .map_or(DRM_DEFAULT_CURSOR_HEIGHT, |size| u64::from(size.height()))
            }
            DrmGetCapability::Addfb2Modifiers => mode_config.supports_fb_modifiers() as u64,
            DrmGetCapability::PageFlipTarget => {
                // TODO: Report support once every CRTC exposes a target-aware page-flip operation.
                0
            }
            DrmGetCapability::CrtcInVblankEvent => 1,
            // TODO: Derive this from atomic CRTC operations
            // once atomic modesetting is implemented.
            DrmGetCapability::AtomicAsyncPageFlip => 0,
            _ => 0,
        };

        Ok(result)
    }

    pub(super) fn drm_auth_magic(&self, cmd: ioctl_defs::AuthMagic) -> Result<i32> {
        let args: DrmAuth = cmd.read()?;
        self.authenticate_magic(args.magic)?;
        Ok(0)
    }

    pub(super) fn drm_set_master(&self, _cmd: ioctl_defs::SetMaster) -> Result<i32> {
        self.set_master()?;
        Ok(0)
    }

    pub(super) fn drm_drop_master(&self, _cmd: ioctl_defs::DropMaster) -> Result<i32> {
        self.drop_master()?;
        Ok(0)
    }

    pub(super) fn drm_wait_vblank(&self, cmd: ioctl_defs::WaitVblank) -> Result<i32> {
        let mut args: DrmWaitVblank = cmd.read()?;
        let request = *args.request();
        let mut flags = DrmWaitVblankFlags::from_bits(request.type_.bits()).ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "the DRM vblank request contains unknown flags",
            )
        })?;

        // Rejects SIGNAL and excludes FLIP from the accepted wait-vblank mask.
        if flags.intersects(DrmWaitVblankFlags::SIGNAL | DrmWaitVblankFlags::FLIP) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the DRM vblank request contains unsupported flags"
            );
        }

        // A nonzero high CRTC index takes precedence over the legacy SECONDARY flag.
        const HIGH_CRTC_SHIFT: u32 = 1;
        let crtc_index = if flags.intersects(DrmWaitVblankFlags::HIGH_CRTC_MASK) {
            KmsObjectIndex::new(
                ((flags & DrmWaitVblankFlags::HIGH_CRTC_MASK).bits() >> HIGH_CRTC_SHIFT) as usize,
            )
        } else {
            KmsObjectIndex::new(flags.contains(DrmWaitVblankFlags::SECONDARY) as usize)
        };

        let kms_ops = self.device().as_kms_ops().ok_or_else(|| {
            Error::with_message(Errno::EOPNOTSUPP, "the DRM device lacks modesetting")
        })?;
        let crtc = {
            let object_store = kms_ops.mode_config().object_store().lock();
            object_store
                .lookup_crtc_by_index(crtc_index)
                .cloned()
                .ok_or(Errno::EINVAL)?
        };

        let current_sequence = crtc.vblank_sequence();
        let mut target_sequence = if flags.contains(DrmWaitVblankFlags::RELATIVE) {
            // Clears RELATIVE because the reply sequence is absolute, not a relative offset.
            flags.remove(DrmWaitVblankFlags::RELATIVE);
            current_sequence.wrapping_add(request.sequence as u64)
        } else {
            // The legacy UAPI supplies only the low 32 bits of an absolute sequence.
            // Reconstruct the nearest 64-bit target using a signed modular offset.
            let offset = request.sequence.wrapping_sub(current_sequence as u32) as i32;
            current_sequence.wrapping_add_signed(offset as i64)
        };

        // Honors NEXT_ON_MISS by moving an already reached target to the next vblank.
        if flags.contains(DrmWaitVblankFlags::NEXT_ON_MISS)
            && current_sequence.wrapping_sub(target_sequence) <= MAX_PASSED_DISTANCE
        {
            // Vblank sequences are modular counters; advancing past u64::MAX wraps to zero.
            target_sequence = current_sequence.wrapping_add(1);
            flags.remove(DrmWaitVblankFlags::NEXT_ON_MISS);
        }

        // Registers an asynchronous notification instead of blocking the ioctl caller.
        if flags.contains(DrmWaitVblankFlags::EVENT) {
            let vblank_event_len = size_of::<DrmVblankEventData>();
            // Reserves capacity before submission, which may complete the event immediately.
            self.event_receiver()
                .reserve_event_space(vblank_event_len)?;

            let vblank_event = DrmVblankEvent::new(
                target_sequence,
                request.signal as u64,
                crtc.id(),
                Arc::downgrade(self.event_receiver()),
            );

            let reply_sequence = crtc.queue_vblank_event(vblank_event).inspect_err(|_| {
                // Returns the reservation only if the event was not accepted.
                self.event_receiver().release_event_space(vblank_event_len);
            })?;
            *args.reply_mut() = DrmWaitVblankReply {
                type_: flags,
                sequence: reply_sequence as u32,
                tval_sec: 0,
                tval_usec: 0,
            };
            cmd.write(&args)?;

            return Ok(0);
        }

        // Blocks until the target vblank sequence is reached, the CRTC is disabled,
        // a signal interrupts, or the wait times out.
        let wait_result = crtc.wait_target_sequence(target_sequence)?;
        let (sequence, timestamp) = match &wait_result {
            DrmVblankWaitResult::Finished {
                sequence,
                timestamp,
            }
            | DrmVblankWaitResult::TimedOut {
                sequence,
                timestamp,
            } => (*sequence, *timestamp),
        };

        *args.reply_mut() = DrmWaitVblankReply {
            type_: flags,
            sequence: sequence as u32,
            tval_sec: timestamp.as_secs() as isize,
            tval_usec: timestamp.subsec_micros() as isize,
        };
        cmd.write(&args)?;

        // Reports a timeout only after writing back the latest sequence and timestamp.
        if matches!(wait_result, DrmVblankWaitResult::TimedOut { .. }) {
            return_errno_with_message!(Errno::EBUSY, "the DRM vblank wait timed out");
        }

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

/// `struct drm_get_cap` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L786>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmGetCap {
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

/// `union drm_wait_vblank` in Linux.
///
/// The ioctl reads its input through `request` and writes its result through `reply`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L507-L515>.
#[pod_union]
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union DrmWaitVblank {
    request: DrmWaitVblankRequest,
    reply: DrmWaitVblankReply,
}

/// `struct drm_wait_vblank_request` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L494-L498>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmWaitVblankRequest {
    type_: DrmWaitVblankFlags,
    sequence: u32,
    signal: usize,
}

/// `struct drm_wait_vblank_reply` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L500-L505>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmWaitVblankReply {
    type_: DrmWaitVblankFlags,
    sequence: u32,
    tval_sec: isize,
    tval_usec: isize,
}

bitflags::bitflags! {
    /// `enum drm_vblank_seq_type` in Linux.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L477-L487>.
    #[repr(transparent)]
    #[derive(Pod)]
    struct DrmWaitVblankFlags: u32 {
        // ABSOLUTE is encoded as zero. If RELATIVE is absent, the request is absolute.
        const RELATIVE       = 0x0000_0001;
        const HIGH_CRTC_MASK = 0x0000_003e;
        const EVENT          = 0x0400_0000;
        const FLIP           = 0x0800_0000;
        const NEXT_ON_MISS   = 0x1000_0000;
        const SECONDARY      = 0x2000_0000;
        const SIGNAL         = 0x4000_0000;
    }
}
