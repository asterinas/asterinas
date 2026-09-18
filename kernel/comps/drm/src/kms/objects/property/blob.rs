// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;

use crate::kms::objects::KmsObjectId;

/// A blob object referenced by a blob-typed DRM property.
///
/// A property blob stores opaque payload bytes,
/// such as mode descriptors or capability blobs.
///
/// In modern DRM semantics, blob-typed properties carry the blob object's ID as their value
/// rather than embedding the blob payload directly in the property attachment.
#[derive(Debug)]
pub(crate) struct DrmPropertyBlob {
    id: KmsObjectId,
    data: Box<[u8]>,
}

impl DrmPropertyBlob {
    pub(crate) fn new(id: KmsObjectId, data: Box<[u8]>) -> Self {
        Self { id, data }
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
