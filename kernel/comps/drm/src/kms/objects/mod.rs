// SPDX-License-Identifier: MPL-2.0

//! KMS objects and their device-local object store.
//!
//! This module defines planes, CRTCs, encoders, connectors, framebuffers,
//! properties, and property blobs.
//! Static objects belonging to a DRM device are registered by type in
//! [`DrmKmsObjectStore`]. Static objects and per-file objects such as
//! framebuffers share a device-local `KmsObjectId` namespace.
//!
//! [`builder::DrmKmsObjectStoreBuilder`] constructs and validates the static KMS
//! topology before materializing the objects in the store.

use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_core::prelude::*;
use aster_util::ranged_integer::RangedU32;
use bitvec::array::BitArray;
use hashbrown::HashMap;
use int_to_c_enum::TryFromInt;
use sparse_id_alloc::SparseIdAlloc;

use crate::kms::objects::{
    connector::{DrmConnector, DrmConnectorConfig},
    crtc::{DrmCrtc, DrmCrtcConfig},
    encoder::{DrmEncoder, DrmEncoderConfig},
    plane::{DrmPlane, DrmPlaneConfig},
};

pub mod builder;
pub mod connector;
pub(crate) mod crtc;
pub mod encoder;
pub(crate) mod framebuffer;
pub mod plane;

pub type KmsObjectId = u32;

pub(crate) const MAX_OBJECTS_PER_TYPE: usize = u32::BITS as usize;

/// A zero-based position in one KMS object type's registration order.
///
/// The index is local to an object type. For example, encoder index 0 and CRTC
/// index 0 refer to entries in different lists.
pub type KmsObjectIndex = RangedU32<0, { MAX_OBJECTS_PER_TYPE as u32 - 1 }>;

type KmsObjectMask = BitArray<[u32; MAX_OBJECTS_PER_TYPE / u32::BITS as usize]>;

/// Registers all KMS objects belonging to a DRM device.
///
/// Static objects are grouped by type and looked up by their device-local
/// `KmsObjectId`. Each object stores its own per-type registration index.
/// The ID allocator is also used for per-file KMS objects, but those objects
/// are owned and tracked outside this store.
#[derive(Debug)]
pub struct DrmKmsObjectStore {
    id_allocator: SparseIdAlloc,
    planes: HashMap<KmsObjectId, Arc<DrmPlane>>,
    crtcs: HashMap<KmsObjectId, Arc<DrmCrtc>>,
    encoders: HashMap<KmsObjectId, Arc<DrmEncoder>>,
    connectors: HashMap<KmsObjectId, Arc<DrmConnector>>,
}

impl Default for DrmKmsObjectStore {
    fn default() -> Self {
        Self {
            id_allocator: SparseIdAlloc::new(1, u32::MAX),
            planes: HashMap::new(),
            crtcs: HashMap::new(),
            encoders: HashMap::new(),
            connectors: HashMap::new(),
        }
    }
}

impl DrmKmsObjectStore {
    pub(crate) fn alloc_object_id(&mut self) -> Result<KmsObjectId> {
        let id = self
            .id_allocator
            .alloc()
            .ok_or_else(|| Error::with_message(Errno::ENOSPC, "all KMS object IDs are in use"))?;

        Ok(id)
    }

    pub(crate) fn free_object_id(&mut self, id: KmsObjectId) {
        self.id_allocator.free(id);
    }

    fn create_plane(
        &mut self,
        index: KmsObjectIndex,
        config: DrmPlaneConfig,
    ) -> Result<Arc<DrmPlane>> {
        let id = self.alloc_object_id()?;
        let plane = Arc::new(DrmPlane::new(id, index, config));

        self.planes.insert(id, plane.clone());
        Ok(plane)
    }

    fn create_crtc(
        &mut self,
        index: KmsObjectIndex,
        config: DrmCrtcConfig,
    ) -> Result<Arc<DrmCrtc>> {
        let id = self.alloc_object_id()?;
        let crtc = Arc::new(DrmCrtc::new(id, index, config));

        self.crtcs.insert(id, crtc.clone());
        Ok(crtc)
    }

    fn create_encoder(
        &mut self,
        index: KmsObjectIndex,
        config: DrmEncoderConfig,
    ) -> Result<Arc<DrmEncoder>> {
        let id = self.alloc_object_id()?;
        let encoder = Arc::new(DrmEncoder::new(id, index, config));

        self.encoders.insert(id, encoder.clone());
        Ok(encoder)
    }

    fn create_connector(
        &mut self,
        index: KmsObjectIndex,
        config: DrmConnectorConfig,
    ) -> Result<Arc<DrmConnector>> {
        let id = self.alloc_object_id()?;
        let connector = Arc::new(DrmConnector::new(id, index, config));

        self.connectors.insert(id, connector.clone());
        Ok(connector)
    }

    /// Collects object IDs in per-type registration-index order.
    ///
    /// Preserving registration order here keeps those array positions aligned with the
    /// mask bits exposed to userspace.
    pub(crate) fn collect_object_ids(&self, type_: DrmKmsObjectType) -> Vec<KmsObjectId> {
        let mut indexed_ids = match type_ {
            DrmKmsObjectType::Crtc => self
                .crtcs
                .values()
                .map(|crtc| (crtc.index(), crtc.id()))
                .collect::<Vec<_>>(),
            DrmKmsObjectType::Connector => self
                .connectors
                .values()
                .map(|connector| (connector.index(), connector.id()))
                .collect::<Vec<_>>(),
            DrmKmsObjectType::Encoder => self
                .encoders
                .values()
                .map(|encoder| (encoder.index(), encoder.id()))
                .collect::<Vec<_>>(),
            DrmKmsObjectType::Plane => self
                .planes
                .values()
                .map(|plane| (plane.index(), plane.id()))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };

        indexed_ids.sort_unstable_by_key(|(index, _)| *index);
        indexed_ids.into_iter().map(|(_, id)| id).collect()
    }

    #[expect(unused)]
    pub(crate) fn get_object_index(
        &self,
        id: KmsObjectId,
        type_: DrmKmsObjectType,
    ) -> Option<KmsObjectIndex> {
        match type_ {
            DrmKmsObjectType::Plane => self.planes.get(&id).map(|plane| plane.index()),
            DrmKmsObjectType::Crtc => self.crtcs.get(&id).map(|crtc| crtc.index()),
            DrmKmsObjectType::Encoder => self.encoders.get(&id).map(|encoder| encoder.index()),
            DrmKmsObjectType::Connector => {
                self.connectors.get(&id).map(|connector| connector.index())
            }
            DrmKmsObjectType::Any => self
                .planes
                .get(&id)
                .map(|plane| plane.index())
                .or_else(|| self.crtcs.get(&id).map(|crtc| crtc.index()))
                .or_else(|| self.encoders.get(&id).map(|encoder| encoder.index()))
                .or_else(|| self.connectors.get(&id).map(|connector| connector.index())),
            _ => None,
        }
    }

    pub(crate) fn get_object_id(
        &self,
        index: KmsObjectIndex,
        type_: DrmKmsObjectType,
    ) -> Option<KmsObjectId> {
        match type_ {
            DrmKmsObjectType::Plane => self
                .planes
                .values()
                .find(|plane| plane.index() == index)
                .map(|plane| plane.id()),
            DrmKmsObjectType::Crtc => self
                .crtcs
                .values()
                .find(|crtc| crtc.index() == index)
                .map(|crtc| crtc.id()),
            DrmKmsObjectType::Encoder => self
                .encoders
                .values()
                .find(|encoder| encoder.index() == index)
                .map(|encoder| encoder.id()),
            DrmKmsObjectType::Connector => self
                .connectors
                .values()
                .find(|connector| connector.index() == index)
                .map(|connector| connector.id()),
            _ => None,
        }
    }

    pub(crate) fn lookup_crtc(&self, id: KmsObjectId) -> Option<&Arc<DrmCrtc>> {
        self.crtcs.get(&id)
    }

    pub(crate) fn lookup_plane(&self, id: KmsObjectId) -> Option<&Arc<DrmPlane>> {
        self.planes.get(&id)
    }

    pub(crate) fn lookup_encoder(&self, id: KmsObjectId) -> Option<&Arc<DrmEncoder>> {
        self.encoders.get(&id)
    }

    pub(crate) fn lookup_connector(&self, id: KmsObjectId) -> Option<&Arc<DrmConnector>> {
        self.connectors.get(&id)
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum DrmKmsObjectType {
    Any = 0,
    Crtc = 0xCCCC_CCCC,
    Connector = 0xC0C0_C0C0,
    Encoder = 0xE0E0_E0E0,
    Mode = 0xDEDE_DEDE,
    Property = 0xB0B0_B0B0,
    Framebuffer = 0xFBFB_FBFB,
    Blob = 0xBBBB_BBBB,
    Plane = 0xEEEE_EEEE,
}
