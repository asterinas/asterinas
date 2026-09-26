// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use crate::utils::DrmDisplayFormat;

const FORMAT_BLOB_CURRENT: u32 = 1;
const DRM_FORMAT_MODIFIER_BLOB_HEADER_SIZE: u32 = 24;

/// Represents a blob object referenced by blob-typed DRM properties.
///
/// A mode blob stores opaque payload bytes, such as mode descriptors or
/// capability blobs. In modern DRM semantics, blob-typed properties carry the
/// blob object's ID as their value rather than embedding the blob payload
/// directly in the property attachment.
#[derive(Debug, Clone)]
pub struct DrmPropertyBlob {
    data: Vec<u8>,
}

impl DrmPropertyBlob {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn encode_in_formats(format_types: &[DrmDisplayFormat]) -> Self {
        let formats_offset = DRM_FORMAT_MODIFIER_BLOB_HEADER_SIZE;
        let modifiers_offset =
            formats_offset + (format_types.len() as u32 * size_of::<u32>() as u32);
        let mut data = Vec::with_capacity(modifiers_offset as usize);

        fn push_u32(blob: &mut Vec<u8>, value: u32) {
            blob.extend_from_slice(&value.to_ne_bytes());
        }

        push_u32(&mut data, FORMAT_BLOB_CURRENT);
        push_u32(&mut data, 0);
        push_u32(&mut data, format_types.len() as u32);
        push_u32(&mut data, formats_offset);
        push_u32(&mut data, 0);
        push_u32(&mut data, modifiers_offset);

        for format in format_types {
            push_u32(&mut data, *format as u32);
        }

        Self { data }
    }
}
