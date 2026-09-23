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

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_core::prelude::*;
use aster_util::ranged_integer::RangedUsize;
use bitvec::array::BitArray;
use hashbrown::HashMap;
use int_to_c_enum::TryFromInt;
use sparse_id_alloc::SparseIdAlloc;

use crate::kms::objects::{
    connector::DrmConnector,
    crtc::DrmCrtc,
    encoder::DrmEncoder,
    plane::DrmPlane,
    property::{DrmProperty, DrmPropertyAttachments, DrmStandardProperty, blob::DrmPropertyBlob},
};

pub mod builder;
pub mod connector;
pub mod crtc;
pub mod encoder;
pub mod framebuffer;
pub mod plane;
pub mod property;

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

    properties: HashMap<KmsObjectId, Arc<DrmProperty>>,
    /// Provides a semantic index for standard properties;
    /// driver-specific properties remain accessible through `properties` by object ID.
    standard_property_ids: HashMap<DrmStandardProperty, KmsObjectId>,
    // TODO: Support both device-lifetime blobs created by the kernel and
    // per-file blobs created and owned by userspace clients.
    property_blobs: HashMap<KmsObjectId, Arc<DrmPropertyBlob>>,
}

impl DrmKmsObjectStore {
    fn new_empty() -> Self {
        Self {
            id_allocator: SparseIdAlloc::new(1, u32::MAX),
            planes: HashMap::new(),
            crtcs: HashMap::new(),
            encoders: HashMap::new(),
            connectors: HashMap::new(),
            properties: HashMap::new(),
            standard_property_ids: HashMap::new(),
            property_blobs: HashMap::new(),
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

    pub(crate) fn lookup_property(&self, id: KmsObjectId) -> Option<&Arc<DrmProperty>> {
        self.properties.get(&id)
    }

    pub(crate) fn lookup_property_blob(&self, id: KmsObjectId) -> Option<&Arc<DrmPropertyBlob>> {
        self.property_blobs.get(&id)
    }

    /// Looks up a KMS mode object's property table without specifying its type.
    ///
    /// The object ID may refer to any KMS mode object,
    /// such as a plane, CRTC, encoder, or connector.
    /// This is an object ID, not a property ID.
    ///
    /// Returns `EINVAL` for an encoder, which has no property table,
    /// and `None` if no supported object matches the ID.
    pub(crate) fn lookup_any_object_properties(
        &self,
        id: KmsObjectId,
    ) -> Result<Option<&DrmPropertyAttachments>> {
        if let Some(plane) = self.lookup_plane_by_id(id) {
            return Ok(Some(plane.properties()));
        }

        if let Some(crtc) = self.lookup_crtc_by_id(id) {
            return Ok(Some(crtc.properties()));
        }

        if let Some(connector) = self.lookup_connector_by_id(id) {
            return Ok(Some(connector.properties()));
        }

        if self.lookup_encoder_by_id(id).is_some() {
            return_errno_with_message!(Errno::EINVAL, "the DRM encoder has no property table");
        }

        Ok(None)
    }

    fn get_or_create_standard_property(
        &mut self,
        standard: DrmStandardProperty,
    ) -> Result<Arc<DrmProperty>> {
        if let Some(property) = self
            .standard_property_ids
            .get(&standard)
            .and_then(|id| self.properties.get(id))
        {
            return Ok(property.clone());
        }

        let id = self.alloc_object_id()?;
        let property = Arc::new(standard.create_property(id));

        let previous = self.properties.insert(id, property.clone());
        debug_assert!(previous.is_none());
        let previous = self.standard_property_ids.insert(standard, id);
        debug_assert!(previous.is_none());
        Ok(property)
    }

    fn create_property_blob(&mut self, data: Box<[u8]>) -> Result<Arc<DrmPropertyBlob>> {
        let id = self.alloc_object_id()?;
        let blob = Arc::new(DrmPropertyBlob::new(id, data));

        let previous = self.property_blobs.insert(id, blob.clone());
        debug_assert!(previous.is_none());
        Ok(blob)
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
