// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_core::prelude::*;

use crate::{
    gem::object::DrmGemObject,
    kms::{objects::KmsObjectId, pixel_format::DrmPixelFormat},
    utils::DrmSize,
};

/// A DRM framebuffer backed by a GEM object.
///
/// The framebuffer describes how pixels are laid out within the object; the
/// GEM object owns the underlying storage.
///
// TODO: Support framebuffers backed by multiple planes.
#[derive(Debug)]
pub(crate) struct DrmFramebuffer {
    id: KmsObjectId,
    size: DrmSize,
    pixel_format: DrmPixelFormat,
    flags: DrmFramebufferFlags,
    pitch: u32,
    offset: u32,
    modifier: u64,
    gem_object: Arc<DrmGemObject>,
}

impl DrmFramebuffer {
    /// Creates a single-plane framebuffer over the supplied GEM object.
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: KmsObjectId,
        size: DrmSize,
        pixel_format: DrmPixelFormat,
        flags: DrmFramebufferFlags,
        pitch: u32,
        offset: u32,
        modifier: u64,
        gem_object: Arc<DrmGemObject>,
    ) -> Result<Self> {
        if size.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the DRM framebuffer size must not be empty");
        }

        // Calculate the minimum GEM object size needed to contain the framebuffer,
        // including its initial offset, row strides, and the last row of pixels.
        let row_size = size.width() as usize * pixel_format.bytes_per_pixel();
        if (pitch as usize) < row_size {
            return_errno_with_message!(
                Errno::EINVAL,
                "the DRM framebuffer pitch is smaller than one pixel row"
            );
        }

        let required_size =
            (size.height() - 1) as usize * pitch as usize + offset as usize + row_size;

        if required_size > gem_object.size() {
            return_errno_with_message!(Errno::EINVAL, "the DRM framebuffer exceeds its GEM object");
        }

        Ok(Self {
            id,
            size,
            pixel_format,
            flags,
            pitch,
            offset,
            modifier,
            gem_object,
        })
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    #[expect(unused)]
    pub(crate) fn size(&self) -> DrmSize {
        self.size
    }

    #[expect(unused)]
    pub(crate) fn pixel_format(&self) -> DrmPixelFormat {
        self.pixel_format
    }

    #[expect(unused)]
    pub(crate) fn flags(&self) -> DrmFramebufferFlags {
        self.flags
    }

    #[expect(unused)]
    pub(crate) fn pitch(&self) -> u32 {
        self.pitch
    }

    #[expect(unused)]
    pub(crate) fn offset(&self) -> u32 {
        self.offset
    }

    #[expect(unused)]
    pub(crate) fn modifier(&self) -> u64 {
        self.modifier
    }

    #[expect(unused)]
    pub(crate) fn gem_object(&self) -> &DrmGemObject {
        &self.gem_object
    }
}

bitflags::bitflags! {
    /// Framebuffer flags exposed through the DRM mode UAPI.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L666-L667>
    pub(crate) struct DrmFramebufferFlags: u32 {
        /// The framebuffer contains interlaced image data.
        const INTERLACED = 1 << 0;
        /// The framebuffer uses an explicitly specified format modifier.
        const MODIFIERS = 1 << 1;
    }
}
