// SPDX-License-Identifier: MPL-2.0

//! KMS objects and their device-local object store.
//!
//! This module defines planes, CRTCs, encoders, connectors, framebuffers, properties, and property blobs.
//!
//! Static objects belonging to a DRM device are registered by type in [`DrmKmsObjectStore`].
//!
//! Static objects and per-file objects such as framebuffers share a device-local `KmsObjectId` namespace.
//!
//! [`builder::DrmKmsObjectStoreBuilder`] constructs and validates the static KMS topology
//! before materializing the objects in the store.

use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_core::prelude::*;
use aster_util::ranged_integer::RangedUsize;
use bitvec::array::BitArray;
use hashbrown::HashMap;
use int_to_c_enum::TryFromInt;
use sparse_id_alloc::SparseIdAlloc;

use crate::kms::objects::{
    connector::DrmConnector, crtc::DrmCrtc, encoder::DrmEncoder, plane::DrmPlane,
};

pub mod builder;
pub mod connector;
pub(crate) mod crtc;
pub mod encoder;
pub(crate) mod framebuffer;
pub mod plane;

pub(crate) type KmsObjectId = u32;

pub(crate) const MAX_OBJECTS_PER_TYPE: usize = u32::BITS as usize;

/// A zero-based position in one KMS object type's registration order.
///
/// The index is local to an object type.
///
/// For example, encoder index 0 and CRTC index 0 refer to entries in different lists.
pub type KmsObjectIndex = RangedUsize<0, { MAX_OBJECTS_PER_TYPE - 1 }>;

type KmsObjectMask = BitArray<[u32; MAX_OBJECTS_PER_TYPE / u32::BITS as usize]>;

/// Registers all KMS objects belonging to a DRM device.
///
/// Static objects are grouped by type and looked up by their device-local `KmsObjectId`.
/// Each object stores its own per-type registration index.
///
/// The ID allocator is also used for per-file KMS objects,
/// but those objects are owned and tracked outside this store.
#[derive(Debug)]
pub struct DrmKmsObjectStore {
    id_allocator: SparseIdAlloc,
    planes: HashMap<KmsObjectId, Arc<DrmPlane>>,
    crtcs: HashMap<KmsObjectId, Arc<DrmCrtc>>,
    encoders: HashMap<KmsObjectId, Arc<DrmEncoder>>,
    connectors: HashMap<KmsObjectId, Arc<DrmConnector>>,
}

impl DrmKmsObjectStore {
    fn new_empty() -> Self {
        Self {
            id_allocator: SparseIdAlloc::new(1, u32::MAX),
            planes: HashMap::new(),
            crtcs: HashMap::new(),
            encoders: HashMap::new(),
            connectors: HashMap::new(),
        }
    }

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

    fn insert_plane(&mut self, plane: Arc<DrmPlane>) {
        self.planes.insert(plane.id(), plane);
    }

    fn insert_crtc(&mut self, crtc: Arc<DrmCrtc>) {
        self.crtcs.insert(crtc.id(), crtc);
    }

    fn insert_encoder(&mut self, encoder: Arc<DrmEncoder>) {
        self.encoders.insert(encoder.id(), encoder);
    }

    fn insert_connector(&mut self, connector: Arc<DrmConnector>) {
        self.connectors.insert(connector.id(), connector);
    }

    pub(crate) fn collect_sorted_crtc_ids(&self) -> Vec<KmsObjectId> {
        let mut indexed_ids = self
            .crtcs()
            .map(|crtc| (crtc.index(), crtc.id()))
            .collect::<Vec<_>>();

        indexed_ids.sort_unstable_by_key(|(index, _)| *index);
        indexed_ids.into_iter().map(|(_, id)| id).collect()
    }

    pub(crate) fn collect_sorted_encoder_ids(&self) -> Vec<KmsObjectId> {
        let mut indexed_ids = self
            .encoders()
            .map(|encoder| (encoder.index(), encoder.id()))
            .collect::<Vec<_>>();

        indexed_ids.sort_unstable_by_key(|(index, _)| *index);
        indexed_ids.into_iter().map(|(_, id)| id).collect()
    }

    pub(crate) fn collect_sorted_connector_ids(&self) -> Vec<KmsObjectId> {
        let mut indexed_ids = self
            .connectors()
            .map(|connector| (connector.index(), connector.id()))
            .collect::<Vec<_>>();

        indexed_ids.sort_unstable_by_key(|(index, _)| *index);
        indexed_ids.into_iter().map(|(_, id)| id).collect()
    }

    pub(crate) fn planes(&self) -> impl Iterator<Item = &Arc<DrmPlane>> {
        self.planes.values()
    }

    pub(crate) fn crtcs(&self) -> impl Iterator<Item = &Arc<DrmCrtc>> {
        self.crtcs.values()
    }

    pub(crate) fn encoders(&self) -> impl Iterator<Item = &Arc<DrmEncoder>> {
        self.encoders.values()
    }

    pub(crate) fn connectors(&self) -> impl Iterator<Item = &Arc<DrmConnector>> {
        self.connectors.values()
    }

    pub(crate) fn lookup_plane_by_id(&self, id: KmsObjectId) -> Option<&Arc<DrmPlane>> {
        self.planes.get(&id)
    }

    pub(crate) fn lookup_crtc_by_id(&self, id: KmsObjectId) -> Option<&Arc<DrmCrtc>> {
        self.crtcs.get(&id)
    }

    pub(crate) fn lookup_encoder_by_id(&self, id: KmsObjectId) -> Option<&Arc<DrmEncoder>> {
        self.encoders.get(&id)
    }

    pub(crate) fn lookup_encoder_by_index(
        &self,
        index: KmsObjectIndex,
    ) -> Option<&Arc<DrmEncoder>> {
        self.encoders().find(|encoder| encoder.index() == index)
    }

    pub(crate) fn lookup_connector_by_id(&self, id: KmsObjectId) -> Option<&Arc<DrmConnector>> {
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
