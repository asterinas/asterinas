// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use aster_core::prelude::*;
use aster_util::fixed_str::FixedCStr;

use crate::utils::DrmSize;

const DRM_DISPLAY_MODE_NAME_LEN: usize = 32;

/// The kernel-internal representation of a display mode.
///
/// It describes the logical scanout timing, synchronization flags, mode
/// origin, and userspace-visible name.
/// Unlike [`DrmModeInfo`], its Rust layout is not part of the userspace ABI.
/// It is converted to or from the UAPI representation when crossing the ioctl
/// boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrmDisplayMode {
    pixel_clock_khz: u32,
    horizontal: DrmModeTiming,
    vertical: DrmModeTiming,
    hskew: u32,
    vscan: u32,
    flags: DrmModeFlag,
    mode_type: DrmModeType,

    name: [u8; DRM_DISPLAY_MODE_NAME_LEN],
}

impl DrmDisplayMode {
    /// Creates a simple fixed-refresh display mode.
    pub fn from_size(size: DrmSize, vrefresh_hz: u32) -> Result<Self> {
        if size.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the display mode size must not be empty");
        }

        let mut name = [0u8; DRM_DISPLAY_MODE_NAME_LEN];
        let formatted_name = format!("{}x{}", size.width(), size.height());
        let formatted_name_bytes = formatted_name.as_bytes();
        let copy_len = formatted_name_bytes
            .len()
            .min(DRM_DISPLAY_MODE_NAME_LEN - 1);
        name[..copy_len].copy_from_slice(&formatted_name_bytes[..copy_len]);

        let pixel_clock_khz = u64::from(size.width())
            .checked_mul(u64::from(size.height()))
            .and_then(|pixels| pixels.checked_mul(u64::from(vrefresh_hz)))
            .map(|pixel_clock_hz| pixel_clock_hz / 1000)
            .and_then(|pixel_clock_khz| u32::try_from(pixel_clock_khz).ok())
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EOVERFLOW,
                    "the calculated display mode clock exceeds the internal limit",
                )
            })?;

        Ok(Self {
            pixel_clock_khz,
            horizontal: DrmModeTiming::from_active(size.width()),
            vertical: DrmModeTiming::from_active(size.height()),
            hskew: 0,
            vscan: 0,
            flags: DrmModeFlag::empty(),
            mode_type: DrmModeType::DRIVER,
            name,
        })
    }

    fn vrefresh(&self) -> Result<u32> {
        if self.horizontal.total == 0 || self.vertical.total == 0 {
            return Ok(0);
        }

        let mut numerator = u128::from(self.pixel_clock_khz);
        let mut denominator = u128::from(self.horizontal.total) * u128::from(self.vertical.total);

        if self.flags.contains(DrmModeFlag::INTERLACE) {
            numerator *= 2;
        }

        if self.flags.contains(DrmModeFlag::DBLSCAN) {
            denominator *= 2;
        }

        if self.vscan > 1 {
            denominator *= u128::from(self.vscan);
        }

        let refresh_hz = (numerator * 1000 + denominator / 2) / denominator;
        u32::try_from(refresh_hz).map_err(|_| {
            Error::with_message(
                Errno::EOVERFLOW,
                "the calculated display mode refresh rate exceeds the DRM UAPI limit",
            )
        })
    }

    pub fn hdisplay(&self) -> u32 {
        self.horizontal.active
    }

    pub fn vdisplay(&self) -> u32 {
        self.vertical.active
    }
}

/// Timing parameters along one axis of a display mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DrmModeTiming {
    active: u32,
    sync_start: u32,
    sync_end: u32,
    total: u32,
}

impl DrmModeTiming {
    fn from_active(active: u32) -> Self {
        Self {
            active,
            sync_start: active,
            sync_end: active,
            total: active,
        }
    }
}

impl TryFrom<(u16, u16, u16, u16)> for DrmModeTiming {
    type Error = Error;

    fn try_from((active, sync_start, sync_end, total): (u16, u16, u16, u16)) -> Result<Self> {
        if active == 0 || active > sync_start || sync_start > sync_end || sync_end > total {
            return_errno_with_message!(Errno::EINVAL, "the display mode timing is invalid");
        }

        Ok(Self {
            active: u32::from(active),
            sync_start: u32::from(sync_start),
            sync_end: u32::from(sync_end),
            total: u32::from(total),
        })
    }
}

bitflags::bitflags! {
    /// Mode origin and preference flags exposed through the DRM mode UAPI.
    ///
    /// These flags are stored in `drm_mode_modeinfo::type`.
    /// Their bit values are part of the userspace ABI and must remain compatible
    /// with Linux.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L49-L59>.
    pub struct DrmModeType: u32 {
        /// Deprecated by Linux; retained for UAPI compatibility.
        const BUILTIN   = 1 << 0;
        /// Deprecated by Linux; retained for UAPI compatibility.
        const CLOCK_C   = (1 << 1) | Self::BUILTIN.bits();
        /// Deprecated by Linux; retained for UAPI compatibility.
        const CRTC_C    = (1 << 2) | Self::BUILTIN.bits();
        const PREFERRED = 1 << 3;
        /// Deprecated by Linux; retained for UAPI compatibility.
        const DEFAULT   = 1 << 4;
        const USERDEF   = 1 << 5;
        const DRIVER    = 1 << 6;
    }
}

bitflags::bitflags! {
    /// Display timing and synchronization flags exposed through the DRM mode UAPI.
    ///
    /// These flags are stored in `drm_mode_modeinfo::flags`.
    /// Bits 0 through 13 are ABI-compatible with the corresponding XRandR mode
    /// flags and must not be changed or reused.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L61-L84>.
    pub struct DrmModeFlag: u32 {
        const PHSYNC    = 1 << 0;
        const NHSYNC    = 1 << 1;
        const PVSYNC    = 1 << 2;
        const NVSYNC    = 1 << 3;
        const INTERLACE = 1 << 4;
        const DBLSCAN   = 1 << 5;
        const CSYNC     = 1 << 6;
        const PCSYNC    = 1 << 7;
        const NCSYNC    = 1 << 8;
        const HSKEW     = 1 << 9;
        /// Deprecated by Linux; retained for UAPI compatibility.
        const BCAST     = 1 << 10;
        /// Deprecated by Linux; retained for UAPI compatibility.
        const PIXMUX    = 1 << 11;
        const DBLCLK    = 1 << 12;
        const CLKDIV2   = 1 << 13;
    }
}

/// DRM display mode.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L221-L260>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct DrmModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,

    vrefresh: u32,

    flags: u32,
    type_: u32,

    name: FixedCStr<DRM_DISPLAY_MODE_NAME_LEN>,
}

impl TryFrom<DrmDisplayMode> for DrmModeInfo {
    type Error = Error;

    fn try_from(display_mode: DrmDisplayMode) -> Result<Self> {
        let to_uapi_timing = |value| {
            u16::try_from(value).map_err(|_| {
                Error::with_message(
                    Errno::EOVERFLOW,
                    "the display mode timing exceeds the DRM UAPI limit",
                )
            })
        };

        Ok(Self {
            clock: display_mode.pixel_clock_khz,
            hdisplay: to_uapi_timing(display_mode.horizontal.active)?,
            hsync_start: to_uapi_timing(display_mode.horizontal.sync_start)?,
            hsync_end: to_uapi_timing(display_mode.horizontal.sync_end)?,
            htotal: to_uapi_timing(display_mode.horizontal.total)?,
            hskew: to_uapi_timing(display_mode.hskew)?,
            vdisplay: to_uapi_timing(display_mode.vertical.active)?,
            vsync_start: to_uapi_timing(display_mode.vertical.sync_start)?,
            vsync_end: to_uapi_timing(display_mode.vertical.sync_end)?,
            vtotal: to_uapi_timing(display_mode.vertical.total)?,
            vscan: to_uapi_timing(display_mode.vscan)?,
            vrefresh: display_mode.vrefresh()?,
            flags: display_mode.flags.bits(),
            type_: display_mode.mode_type.bits(),
            name: FixedCStr::from_bytes_until_nul(&display_mode.name),
        })
    }
}
