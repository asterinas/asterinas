// SPDX-License-Identifier: MPL-2.0

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::fmt::Debug;

use aster_core::prelude::*;
use aster_util::ranged_integer::RangedUsize;
use bitvec::array::BitArray;
use hashbrown::HashMap;
use int_to_c_enum::TryFromInt;
use ostd::sync::Mutex;
use sparse_id_alloc::SparseIdAlloc;

use crate::kms::objects::{
    connector::DrmConnector,
    crtc::DrmCrtc,
    encoder::DrmEncoder,
    framebuffer::DrmFramebuffer,
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

/// The object store and shared ID namespace for a KMS device.
///
/// `KmsStaticObjects` owns static objects, and `KmsDynamicObjects` weakly indexes dynamic objects.
/// Both share the device's `u32` ID namespace.
#[derive(Debug)]
pub struct KmsObjectStore {
    id_allocator: Arc<Mutex<SparseIdAlloc>>,
    static_objects: KmsStaticObjects,
    dynamic_objects: Arc<Mutex<KmsDynamicObjects>>,
}

impl KmsObjectStore {
    pub(crate) fn alloc_object_id(&self) -> Result<OwnedKmsObjectId> {
        OwnedKmsObjectId::alloc(&self.id_allocator)
    }

    pub fn static_objects(&self) -> &KmsStaticObjects {
        &self.static_objects
    }

    pub(crate) fn dynamic_objects(&self) -> &Arc<Mutex<KmsDynamicObjects>> {
        &self.dynamic_objects
    }

    pub(crate) fn lookup_property_blob(&self, id: KmsObjectId) -> Option<Arc<DrmPropertyBlob>> {
        self.static_objects
            .lookup_property_blob(id)
            .cloned()
            .or_else(|| self.dynamic_objects.lock().lookup_property_blob(id))
    }
}

/// Owns the KMS objects created during device initialization.
/// Keeps these objects available for the lifetime of the device.
#[derive(Debug, Default)]
pub struct KmsStaticObjects {
    planes: HashMap<KmsObjectId, Arc<DrmPlane>>,
    crtcs: HashMap<KmsObjectId, Arc<DrmCrtc>>,
    encoders: HashMap<KmsObjectId, Arc<DrmEncoder>>,
    connectors: HashMap<KmsObjectId, Arc<DrmConnector>>,
    properties: HashMap<KmsObjectId, Arc<DrmProperty>>,
    /// Provides a semantic index for standard properties;
    /// driver-specific properties remain accessible through `properties` by object ID.
    standard_property_ids: HashMap<DrmStandardProperty, KmsObjectId>,
    /// Device-lifetime property blobs created while building the object store.
    property_blobs: HashMap<KmsObjectId, Arc<DrmPropertyBlob>>,
}

impl KmsStaticObjects {
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

    pub fn lookup_crtc_by_index(&self, index: KmsObjectIndex) -> Option<&Arc<DrmCrtc>> {
        self.crtcs().find(|crtc| crtc.index() == index)
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
            return Err(errno!(EINVAL, "the DRM encoder has no property table"));
        }

        Ok(None)
    }

    fn lookup_standard_property(&self, standard: DrmStandardProperty) -> Option<&Arc<DrmProperty>> {
        self.standard_property_ids
            .get(&standard)
            .and_then(|id| self.properties.get(id))
    }

    fn insert_standard_property(
        &mut self,
        standard: DrmStandardProperty,
        property: Arc<DrmProperty>,
    ) {
        let id = property.id();
        self.properties.insert(id, property);
        self.standard_property_ids.insert(standard, id);
    }

    fn insert_property_blob(&mut self, blob: Arc<DrmPropertyBlob>) {
        self.property_blobs.insert(blob.id(), blob);
    }
}

/// Indexes dynamically created KMS objects by ID using weak references.
/// Open DRM files and active users hold the strong references.
#[derive(Debug, Default)]
pub(crate) struct KmsDynamicObjects {
    framebuffers: HashMap<KmsObjectId, Weak<DrmFramebuffer>>,
    property_blobs: HashMap<KmsObjectId, Weak<DrmPropertyBlob>>,
}

impl KmsDynamicObjects {
    pub(crate) fn add_framebuffer(&mut self, framebuffer: &Arc<DrmFramebuffer>) {
        self.framebuffers
            .insert(framebuffer.id(), Arc::downgrade(framebuffer));
    }

    pub(crate) fn lookup_framebuffer(&self, id: KmsObjectId) -> Option<Arc<DrmFramebuffer>> {
        self.framebuffers.get(&id)?.upgrade()
    }

    fn remove_framebuffer(&mut self, id: KmsObjectId) {
        self.framebuffers.remove(&id);
    }

    pub(crate) fn add_property_blob(&mut self, blob: &Arc<DrmPropertyBlob>) {
        self.property_blobs.insert(blob.id(), Arc::downgrade(blob));
    }

    fn lookup_property_blob(&self, id: KmsObjectId) -> Option<Arc<DrmPropertyBlob>> {
        self.property_blobs.get(&id)?.upgrade()
    }

    fn remove_property_blob(&mut self, id: KmsObjectId) {
        self.property_blobs.remove(&id);
    }
}

/// An owned device-local object ID.
///
/// Dropping it releases the ID if the allocator is still alive.
#[derive(Debug)]
pub(crate) struct OwnedKmsObjectId {
    value: KmsObjectId,
    allocator: Weak<Mutex<SparseIdAlloc>>,
}

impl OwnedKmsObjectId {
    fn value(&self) -> KmsObjectId {
        self.value
    }

    fn alloc(allocator: &Arc<Mutex<SparseIdAlloc>>) -> Result<Self> {
        let value = allocator
            .lock()
            .alloc()
            .ok_or_else(|| errno!(ENOSPC, "all KMS object IDs are in use"))?;
        Ok(Self {
            value,
            allocator: Arc::downgrade(allocator),
        })
    }
}

impl Drop for OwnedKmsObjectId {
    fn drop(&mut self) {
        if let Some(allocator) = self.allocator.upgrade() {
            allocator.lock().free(self.value);
        }
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
