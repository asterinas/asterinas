// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};

use ostd::sync::Mutex;

use crate::kms::objects::{KmsDynamicObjects, KmsObjectId, OwnedKmsObjectId};

/// A blob object referenced by a blob-typed DRM property.
///
/// A property blob stores opaque payload bytes,
/// such as mode descriptors or capability blobs.
///
/// In modern DRM semantics, blob-typed properties carry the blob object's ID as their value
/// rather than embedding the blob payload directly in the property attachment.
#[derive(Debug)]
pub(crate) struct DrmPropertyBlob {
    id: OwnedKmsObjectId,
    data: Box<[u8]>,
    // Only dynamically created blobs need to unregister from this index on drop.
    dynamic_objects: Weak<Mutex<KmsDynamicObjects>>,
}

impl DrmPropertyBlob {
    pub(crate) fn new_static(id: OwnedKmsObjectId, data: Box<[u8]>) -> Self {
        Self {
            id,
            data,
            dynamic_objects: Weak::new(),
        }
    }

    pub(crate) fn new_dynamic(
        id: OwnedKmsObjectId,
        data: Box<[u8]>,
        dynamic_objects: &Arc<Mutex<KmsDynamicObjects>>,
    ) -> Self {
        Self {
            id,
            data,
            dynamic_objects: Arc::downgrade(dynamic_objects),
        }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id.value()
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn len(&self) -> usize {
        self.data.len()
    }
}

impl Drop for DrmPropertyBlob {
    fn drop(&mut self) {
        if let Some(dynamic_objects) = self.dynamic_objects.upgrade() {
            dynamic_objects.lock().remove_property_blob(self.id());
        }
    }
}
