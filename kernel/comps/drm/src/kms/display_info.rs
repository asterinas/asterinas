// SPDX-License-Identifier: MPL-2.0

use aster_core::prelude::*;
use int_to_c_enum::TryFromInt;

use crate::utils::DrmSize;

/// Runtime information about the display sink attached to a connector.
///
/// This information is normally obtained while probing the connector and
/// describes physical characteristics of the sink.
#[derive(Clone, Copy, Debug)]
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
/// exposed through `drm_mode_get_connector::subpixel`.
/// The numeric values are therefore part of the DRM userspace ABI and must
/// remain Linux-compatible.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_connector.h#L141-L149>.
#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub enum SubpixelOrder {
    Unknown = 0,
    HorizontalRgb = 1,
    HorizontalBgr = 2,
    VerticalRgb = 3,
    VerticalBgr = 4,
    None = 5,
}
