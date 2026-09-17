// SPDX-License-Identifier: MPL-2.0

use alloc::format;
use core::ops::RangeInclusive;

use aster_core::prelude::*;

pub(super) const DRM_DISPLAY_MODE_NAME_LEN: usize = 32;

/// A two-dimensional size in pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmSize {
    width: u32,
    height: u32,
}

impl DrmSize {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Returns whether the size is within the given inclusive limits.
    pub fn is_within(
        &self,
        width_range: RangeInclusive<u32>,
        height_range: RangeInclusive<u32>,
    ) -> bool {
        width_range.contains(&self.width) && height_range.contains(&self.height)
    }
}

/// Rectangles are checked by their right/bottom edges:
///
/// ```text
/// (x, y)        width        right = x + width
///    +-------------------------+
///    |                         |
///    |                         | height
///    |                         |
///    +-------------------------+
///                            bottom = y + height
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmRect {
    x: u32,
    y: u32,
    size: DrmSize,
}

impl DrmRect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            size: DrmSize::new(width, height),
        }
    }

    pub fn x(&self) -> u32 {
        self.x
    }

    pub fn y(&self) -> u32 {
        self.y
    }

    pub fn size(&self) -> DrmSize {
        self.size
    }

    pub fn width(&self) -> u32 {
        self.size.width()
    }

    pub fn height(&self) -> u32 {
        self.size.height()
    }

    pub fn is_empty(&self) -> bool {
        self.size.is_empty()
    }

    pub fn right(&self) -> Option<u32> {
        self.x.checked_add(self.size.width())
    }

    pub fn bottom(&self) -> Option<u32> {
        self.y.checked_add(self.size.height())
    }

    /// Returns whether the point is inside the rectangle.
    ///
    /// The left/top edges are inclusive and the right/bottom edges are exclusive.
    pub fn contains_point(&self, x: u32, y: u32) -> bool {
        let Some(right) = self.right() else {
            return false;
        };
        let Some(bottom) = self.bottom() else {
            return false;
        };

        (self.x..right).contains(&x) && (self.y..bottom).contains(&y)
    }

    /// Returns whether `other` is fully contained within `self`.
    pub fn contains_rect(&self, other: &Self) -> bool {
        let Some(self_right) = self.right() else {
            return false;
        };
        let Some(self_bottom) = self.bottom() else {
            return false;
        };
        let Some(other_right) = other.right() else {
            return false;
        };
        let Some(other_bottom) = other.bottom() else {
            return false;
        };

        self.x <= other.x
            && other_right <= self_right
            && self.y <= other.y
            && other_bottom <= self_bottom
    }

    pub fn set_x(&mut self, x: u32) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: u32) {
        self.y = y;
    }

    pub fn set_width(&mut self, width: u32) {
        self.size.width = width;
    }

    pub fn set_height(&mut self, height: u32) {
        self.size.height = height;
    }
}

/// The kernel-internal representation of a display mode.
///
/// It describes the logical scanout timing, synchronization flags, mode
/// origin, and userspace-visible name. Unlike [`DrmModeModeInfo`], its Rust
/// layout is not part of the userspace ABI. It is converted to or from the
/// UAPI representation when crossing the ioctl boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmDisplayMode {
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

    flags: u32,
    type_: u32,

    name: [u8; DRM_DISPLAY_MODE_NAME_LEN],
}

impl DrmDisplayMode {
    /// Creates a simple fixed-refresh display mode.
    pub fn from_size(size: DrmSize, vrefresh_hz: u32) -> Result<Self> {
        let width = u16::try_from(size.width()).map_err(|_| {
            Error::with_message(
                Errno::EINVAL,
                "the display mode width exceeds the DRM UAPI limit",
            )
        })?;
        let height = u16::try_from(size.height()).map_err(|_| {
            Error::with_message(
                Errno::EINVAL,
                "the display mode height exceeds the DRM UAPI limit",
            )
        })?;

        let mut name = [0u8; DRM_DISPLAY_MODE_NAME_LEN];
        let formatted_name = format!("{width}x{height}");
        let formatted_name_bytes = formatted_name.as_bytes();
        let copy_len = formatted_name_bytes
            .len()
            .min(DRM_DISPLAY_MODE_NAME_LEN - 1);
        name[..copy_len].copy_from_slice(&formatted_name_bytes[..copy_len]);

        let clock = u64::from(width) * u64::from(height) * u64::from(vrefresh_hz) / 1000;
        let clock = u32::try_from(clock).map_err(|_| {
            Error::with_message(
                Errno::EOVERFLOW,
                "the calculated display mode clock exceeds the DRM UAPI limit",
            )
        })?;

        Ok(Self {
            clock,
            hdisplay: width,
            hsync_start: width,
            hsync_end: width,
            htotal: width,
            hskew: 0,
            vdisplay: height,
            vsync_start: height,
            vsync_end: height,
            vtotal: height,
            vscan: 0,
            flags: 0,
            type_: DrmModeType::DRIVER.bits(),
            name,
        })
    }

    fn vrefresh(&self) -> u32 {
        if self.htotal == 0 || self.vtotal == 0 {
            return 0;
        }

        let mut num = self.clock as u64;
        let mut den = (self.htotal as u64) * (self.vtotal as u64);

        let flags = DrmModeFlag::from_bits_truncate(self.flags);

        if flags.contains(DrmModeFlag::INTERLACE) {
            num *= 2;
        }

        if flags.contains(DrmModeFlag::DBLSCAN) {
            den *= 2;
        }

        if self.vscan > 1 {
            den *= self.vscan as u64;
        }

        ((num * 1000 + den / 2) / den) as u32
    }

    pub fn hdisplay(&self) -> u16 {
        self.hdisplay
    }

    pub fn vdisplay(&self) -> u16 {
        self.vdisplay
    }
}

bitflags::bitflags! {
    /// Mode origin and preference flags exposed through the DRM mode UAPI.
    ///
    /// These flags are stored in `drm_mode_modeinfo::type`. Their bit values
    /// are part of the userspace ABI and must remain compatible with Linux.
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
    /// These flags are stored in `drm_mode_modeinfo::flags`. Bits 0 through 13
    /// are ABI-compatible with the corresponding XRandR mode flags and must not
    /// be changed or reused.
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

/// Runtime information about the display sink attached to a connector.
///
/// This information is normally obtained while probing the connector and
/// describes physical characteristics of the sink.
#[derive(Debug, Clone, Copy)]
pub struct DrmDisplayInfo {
    mm_width: u32,
    mm_height: u32,
    subpixel_order: SubpixelOrder,
}

impl DrmDisplayInfo {
    pub fn from_dpi(resolution: DrmSize, dpi: u32, subpixel_order: SubpixelOrder) -> Result<Self> {
        if dpi == 0 {
            return_errno_with_message!(Errno::EINVAL, "the display DPI must be nonzero");
        }

        fn pixels_to_mm(pixels: u32, dpi: u32) -> Result<u32> {
            // One inch is exactly 25.4 millimeters. The calculation uses
            // tenths of a millimeter to avoid floating-point arithmetic.
            const MILLIMETERS_PER_INCH_TIMES_TEN: u64 = 254;
            const SCALE: u64 = 10;

            let millimeters =
                u64::from(pixels) * MILLIMETERS_PER_INCH_TIMES_TEN / (u64::from(dpi) * SCALE);

            u32::try_from(millimeters).map_err(|_| {
                Error::with_message(
                    Errno::EOVERFLOW,
                    "the calculated display dimension is too large",
                )
            })
        }

        Ok(Self {
            mm_width: pixels_to_mm(resolution.width(), dpi)?,
            mm_height: pixels_to_mm(resolution.height(), dpi)?,
            subpixel_order,
        })
    }

    pub fn mm_width(&self) -> u32 {
        self.mm_width
    }

    pub fn mm_height(&self) -> u32 {
        self.mm_height
    }

    pub fn subpixel_order(&self) -> u32 {
        self.subpixel_order as u32
    }
}

impl Default for DrmDisplayInfo {
    fn default() -> Self {
        Self {
            mm_width: 0,
            mm_height: 0,
            subpixel_order: SubpixelOrder::Unknown,
        }
    }
}

/// The physical subpixel arrangement of a display panel.
///
/// Although Linux defines this as a kernel-side enum, its discriminant is
/// exposed through `drm_mode_get_connector::subpixel`. The numeric values are
/// therefore part of the DRM userspace ABI and must remain Linux-compatible.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_connector.h#L141-L149>.
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum SubpixelOrder {
    Unknown = 0,
    HorizontalRgb = 1,
    HorizontalBgr = 2,
    VerticalRgb = 3,
    VerticalBgr = 4,
    None = 5,
}

impl TryFrom<u32> for SubpixelOrder {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::HorizontalRgb),
            2 => Ok(Self::HorizontalBgr),
            3 => Ok(Self::VerticalRgb),
            4 => Ok(Self::VerticalBgr),
            5 => Ok(Self::None),
            _ => return_errno_with_message!(Errno::EINVAL, "invalid DRM subpixel order"),
        }
    }
}

/// The userspace ABI representation of a DRM display mode.
///
/// Its field order, integer widths, alignment, flag values, and name length
/// must remain compatible with Linux's `struct drm_mode_modeinfo`. Internal
/// display modes should be converted to this structure only at the ioctl
/// boundary.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L221-L260>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeModeInfo {
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

    name: [u8; DRM_DISPLAY_MODE_NAME_LEN],
}

impl From<DrmDisplayMode> for DrmModeModeInfo {
    fn from(display_mode: DrmDisplayMode) -> Self {
        Self {
            clock: display_mode.clock,
            hdisplay: display_mode.hdisplay,
            hsync_start: display_mode.hsync_start,
            hsync_end: display_mode.hsync_end,
            htotal: display_mode.htotal,
            hskew: display_mode.hskew,
            vdisplay: display_mode.vdisplay,
            vsync_start: display_mode.vsync_start,
            vsync_end: display_mode.vsync_end,
            vtotal: display_mode.vtotal,
            vscan: display_mode.vscan,
            vrefresh: display_mode.vrefresh(),
            flags: display_mode.flags,
            type_: display_mode.type_,
            name: display_mode.name,
        }
    }
}

/// Encodes four bytes as a Linux DRM FOURCC pixel-format identifier.
///
/// Each byte occupies eight bits of the resulting value, with the first byte
/// placed in the least-significant bits.
const fn fourcc_code(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

/// A DRM framebuffer pixel format supported by the kernel.
///
/// Each discriminant is a Linux DRM FOURCC identifier exposed through the
/// userspace API. The numeric values must remain compatible with the
/// corresponding `DRM_FORMAT_*` definitions. This enum contains only the
/// currently supported subset of DRM pixel formats.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_fourcc.h#L113>.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrmDisplayFormat {
    XRGB8888 = fourcc_code(b'X', b'R', b'2', b'4'),
    ARGB8888 = fourcc_code(b'A', b'R', b'2', b'4'),
    XBGR8888 = fourcc_code(b'X', b'B', b'2', b'4'),
    RGBX8888 = fourcc_code(b'R', b'X', b'2', b'4'),
    BGRX8888 = fourcc_code(b'B', b'X', b'2', b'4'),
    C8 = fourcc_code(b'C', b'8', b' ', b' '),
    XRGB1555 = fourcc_code(b'X', b'R', b'1', b'5'),
    RGB565 = fourcc_code(b'R', b'G', b'1', b'6'),
    RGB888 = fourcc_code(b'R', b'G', b'2', b'4'),
    BGR888 = fourcc_code(b'B', b'G', b'2', b'4'),
    XRGB2101010 = fourcc_code(b'X', b'R', b'3', b'0'),
}

impl DrmDisplayFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::C8 => 1,
            Self::XRGB1555 | Self::RGB565 => 2,
            Self::RGB888 | Self::BGR888 => 3,
            Self::XRGB8888
            | Self::ARGB8888
            | Self::XBGR8888
            | Self::RGBX8888
            | Self::BGRX8888
            | Self::XRGB2101010 => 4,
        }
    }
}

impl TryFrom<u32> for DrmDisplayFormat {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            x if x == Self::XRGB8888 as u32 => Ok(Self::XRGB8888),
            x if x == Self::ARGB8888 as u32 => Ok(Self::ARGB8888),
            x if x == Self::XBGR8888 as u32 => Ok(Self::XBGR8888),
            x if x == Self::RGBX8888 as u32 => Ok(Self::RGBX8888),
            x if x == Self::BGRX8888 as u32 => Ok(Self::BGRX8888),
            x if x == Self::C8 as u32 => Ok(Self::C8),
            x if x == Self::XRGB1555 as u32 => Ok(Self::XRGB1555),
            x if x == Self::RGB565 as u32 => Ok(Self::RGB565),
            x if x == Self::RGB888 as u32 => Ok(Self::RGB888),
            x if x == Self::BGR888 as u32 => Ok(Self::BGR888),
            x if x == Self::XRGB2101010 as u32 => Ok(Self::XRGB2101010),
            _ => return_errno_with_message!(Errno::EINVAL, "the DRM display format is unsupported"),
        }
    }
}
