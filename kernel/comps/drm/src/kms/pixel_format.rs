// SPDX-License-Identifier: MPL-2.0

use aster_core::prelude::*;

/// Encodes four bytes as a Linux DRM FOURCC pixel-format identifier.
///
/// Each byte occupies eight bits of the resulting value,
/// with the first byte placed in the least-significant bits.
const fn fourcc_code(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

/// A DRM framebuffer pixel format supported by the kernel.
///
/// Each discriminant is a Linux DRM FOURCC identifier exposed through the userspace API.
/// The numeric values must remain compatible with the corresponding `DRM_FORMAT_*` definitions.
/// This enum contains only the currently supported subset of DRM pixel formats.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_fourcc.h#L113>.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrmPixelFormat {
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

impl DrmPixelFormat {
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

impl TryFrom<(u32, u32)> for DrmPixelFormat {
    type Error = Error;

    fn try_from(value: (u32, u32)) -> Result<Self, Self::Error> {
        match value {
            (8, 8) => Ok(DrmPixelFormat::C8),
            (16, 15) => Ok(DrmPixelFormat::XRGB1555),
            (16, 16) => Ok(DrmPixelFormat::RGB565),
            (24, 24) => Ok(DrmPixelFormat::RGB888),
            (32, 24) => Ok(DrmPixelFormat::XRGB8888),
            (32, 30) => Ok(DrmPixelFormat::XRGB2101010),
            (32, 32) => Ok(DrmPixelFormat::ARGB8888),
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "the legacy DRM framebuffer format is unsupported"
            ),
        }
    }
}
