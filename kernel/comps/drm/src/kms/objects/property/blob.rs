// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec::Vec};

use ostd_pod::IntoBytes;

use crate::kms::{objects::KmsObjectId, pixel_format::DrmPixelFormat};

const FORMAT_BLOB_CURRENT: u32 = 1;

/// A blob object referenced by a blob-typed DRM property.
///
/// A property blob stores opaque payload bytes, such as mode descriptors or
/// capability blobs.
/// In modern DRM semantics, blob-typed properties carry the blob object's ID as
/// their value rather than embedding the blob payload directly in the property
/// attachment.
#[derive(Debug)]
pub(crate) struct DrmPropertyBlob {
    id: KmsObjectId,
    data: Box<[u8]>,
}

impl DrmPropertyBlob {
    pub(crate) fn new(id: KmsObjectId, data: Vec<u8>) -> Self {
        Self {
            id,
            data: data.into_boxed_slice(),
        }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn len(&self) -> usize {
        self.data.len()
    }
}

pub(crate) fn encode_in_formats_blob_data(pixel_formats: &[DrmPixelFormat]) -> Vec<u8> {
    let formats_offset = size_of::<DrmFormatModifierBlobHeader>() as u32;
    let modifiers_offset = formats_offset + (pixel_formats.len() as u32 * size_of::<u32>() as u32);
    let mut data = Vec::with_capacity(modifiers_offset as usize);

    let header = DrmFormatModifierBlobHeader {
        version: FORMAT_BLOB_CURRENT,
        flags: 0,
        count_formats: pixel_formats.len() as u32,
        formats_offset,
        count_modifiers: 0,
        modifiers_offset,
    };
    data.extend_from_slice(header.as_bytes());

    for pixel_format in pixel_formats {
        data.extend_from_slice(&(*pixel_format as u32).to_ne_bytes());
    }

    data
}

/// `struct drm_format_modifier_blob` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L1163-L1185>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct DrmFormatModifierBlobHeader {
    version: u32,
    flags: u32,
    count_formats: u32,
    formats_offset: u32,
    count_modifiers: u32,
    modifiers_offset: u32,
}
