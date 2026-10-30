// SPDX-License-Identifier: MPL-2.0

//! KMS objects, their static topology, and device-local ID registry.
//!
//! [`DrmObjectTopology`] owns static objects; [`DrmObjectRegistry`] weakly indexes dynamic objects.
//! Both share the device's `u32` ID namespace.

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

pub(crate) const MAX_OBJECTS_PER_TYPE: usize = u32::BITS as usize;

/// A zero-based position in one KMS object type's registration order.
///
/// The index is local to an object type.
///
/// For example, encoder index 0 and CRTC index 0 refer to entries in different lists.
pub type DrmKmsObjectIndex = RangedUsize<0, { MAX_OBJECTS_PER_TYPE - 1 }>;
type DrmKmsObjectMask = BitArray<[u32; MAX_OBJECTS_PER_TYPE / u32::BITS as usize]>;

/// Owns a DRM device's static topology and shared object registry.
#[derive(Debug)]
pub struct DrmKmsObjectStore {
    registry: Arc<Mutex<DrmObjectRegistry>>,
    topology: DrmObjectTopology,
}

impl DrmKmsObjectStore {
    fn new(registry: Arc<Mutex<DrmObjectRegistry>>, topology: DrmObjectTopology) -> Self {
        Self { registry, topology }
    }

    pub(crate) fn topology(&self) -> &DrmObjectTopology {
        &self.topology
    }

    pub(crate) fn alloc_object_id(&self) -> Result<DrmKmsObjectId> {
        DrmKmsObjectId::alloc(&self.registry)
    }

    pub(crate) fn add_framebuffer(&self, framebuffer: &Arc<DrmFramebuffer>) {
        self.registry
            .lock()
            .framebuffers
            .insert(framebuffer.id(), Arc::downgrade(framebuffer));
    }

    pub(crate) fn lookup_framebuffer_by_id(&self, id: u32) -> Option<Arc<DrmFramebuffer>> {
        let registry = self.registry.lock();
        registry.framebuffers.get(&id)?.upgrade()
    }

    pub(crate) fn add_property_blob(&self, property_blob: &Arc<DrmPropertyBlob>) {
        self.registry
            .lock()
            .property_blobs
            .insert(property_blob.id(), Arc::downgrade(property_blob));
    }
}

/// Device-wide ID allocation and weak indexes for dynamic objects.
#[derive(Debug)]
pub struct DrmObjectRegistry {
    id_allocator: SparseIdAlloc,
    framebuffers: HashMap<u32, Weak<DrmFramebuffer>>,
    property_blobs: HashMap<u32, Weak<DrmPropertyBlob>>,
}

impl Default for DrmObjectRegistry {
    fn default() -> Self {
        Self {
            id_allocator: SparseIdAlloc::new(1, u32::MAX),
            framebuffers: HashMap::new(),
            property_blobs: HashMap::new(),
        }
    }
}

/// The static KMS objects belonging to a DRM device.
#[derive(Debug, Default)]
pub struct DrmObjectTopology {
    planes: HashMap<u32, Arc<DrmPlane>>,
    crtcs: HashMap<u32, Arc<DrmCrtc>>,
    encoders: HashMap<u32, Arc<DrmEncoder>>,
    connectors: HashMap<u32, Arc<DrmConnector>>,
    properties: HashMap<u32, Arc<DrmProperty>>,
    /// Provides a semantic index for standard properties;
    /// driver-specific properties remain accessible through `properties` by object ID.
    standard_property_ids: HashMap<DrmStandardProperty, u32>,
    /// Device-lifetime property blobs created during topology construction.
    property_blobs: HashMap<u32, Arc<DrmPropertyBlob>>,
}

impl DrmObjectTopology {
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

    pub(crate) fn collect_sorted_crtc_ids(&self) -> Vec<u32> {
        let mut indexed_ids = self
            .crtcs()
            .map(|crtc| (crtc.index(), crtc.id()))
            .collect::<Vec<_>>();

        indexed_ids.sort_unstable_by_key(|(index, _)| *index);
        indexed_ids.into_iter().map(|(_, id)| id).collect()
    }

    pub(crate) fn collect_sorted_encoder_ids(&self) -> Vec<u32> {
        let mut indexed_ids = self
            .encoders()
            .map(|encoder| (encoder.index(), encoder.id()))
            .collect::<Vec<_>>();

        indexed_ids.sort_unstable_by_key(|(index, _)| *index);
        indexed_ids.into_iter().map(|(_, id)| id).collect()
    }

    pub(crate) fn collect_sorted_connector_ids(&self) -> Vec<u32> {
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

    pub(crate) fn lookup_plane_by_id(&self, id: u32) -> Option<&Arc<DrmPlane>> {
        self.planes.get(&id)
    }

    pub(crate) fn lookup_crtc_by_id(&self, id: u32) -> Option<&Arc<DrmCrtc>> {
        self.crtcs.get(&id)
    }

    pub(crate) fn lookup_encoder_by_id(&self, id: u32) -> Option<&Arc<DrmEncoder>> {
        self.encoders.get(&id)
    }

    pub(crate) fn lookup_encoder_by_index(
        &self,
        index: DrmKmsObjectIndex,
    ) -> Option<&Arc<DrmEncoder>> {
        self.encoders().find(|encoder| encoder.index() == index)
    }

    pub(crate) fn lookup_connector_by_id(&self, id: u32) -> Option<&Arc<DrmConnector>> {
        self.connectors.get(&id)
    }

    pub(crate) fn lookup_property(&self, id: u32) -> Option<&Arc<DrmProperty>> {
        self.properties.get(&id)
    }

    pub(crate) fn lookup_property_blob(&self, id: u32) -> Option<&Arc<DrmPropertyBlob>> {
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
        id: u32,
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

/// An owned device-local object ID.
///
/// Dropping it removes any dynamic index entry and releases the ID.
#[derive(Debug)]
pub(crate) struct DrmKmsObjectId {
    value: u32,
    registry: Arc<Mutex<DrmObjectRegistry>>,
}

impl DrmKmsObjectId {
    fn alloc(registry: &Arc<Mutex<DrmObjectRegistry>>) -> Result<Self> {
        let value =
            registry.lock().id_allocator.alloc().ok_or_else(|| {
                Error::with_message(Errno::ENOSPC, "all KMS object IDs are in use")
            })?;

        Ok(Self {
            value,
            registry: registry.clone(),
        })
    }

    fn value(&self) -> u32 {
        self.value
    }
}

impl Drop for DrmKmsObjectId {
    fn drop(&mut self) {
        let mut registry = self.registry.lock();

        registry.framebuffers.remove(&self.value);
        registry.property_blobs.remove(&self.value);
        registry.id_allocator.free(self.value);
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
