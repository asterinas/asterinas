// SPDX-License-Identifier: MPL-2.0

//! KMS objects and their device-local object store.
//!
//! This module defines planes, CRTCs, encoders, connectors, framebuffers,
//! properties, and property blobs.
//! All objects belonging to a DRM device are owned by [`DrmKmsObjectStore`]
//! and share a device-local [`KmsObjectId`] namespace.
//!
//! [`builder::DrmKmsObjectBuilder`] constructs and validates the static KMS
//! topology before materializing the objects in the store.

use alloc::vec::Vec;
use core::fmt::Debug;

use aster_core::prelude::*;
use hashbrown::HashMap;
use sparse_id_alloc::SparseIdAlloc;

use crate::kms::objects::{
    connector::DrmConnector, crtc::DrmCrtc, encoder::DrmEncoder, plane::DrmPlane,
};

pub mod builder;
pub mod connector;
pub mod crtc;
pub mod encoder;
pub mod plane;

pub type KmsObjectId = u32;
pub type KmsObjectIndex = usize;

/// Owns all KMS objects registered for a DRM device.
///
/// Objects share a common ID namespace and can be looked up by their
/// [`KmsObjectId`]. Per-type ID lists preserve registration order and support
/// enumeration and topology index-to-ID conversion.
#[derive(Debug)]
pub struct DrmKmsObjectStore {
    plane_ids: Vec<KmsObjectId>,
    crtc_ids: Vec<KmsObjectId>,
    encoder_ids: Vec<KmsObjectId>,
    connector_ids: Vec<KmsObjectId>,
    object_by_id: HashMap<KmsObjectId, DrmKmsObject>,
    id_allocator: SparseIdAlloc,
}

impl Default for DrmKmsObjectStore {
    fn default() -> Self {
        Self {
            plane_ids: Vec::new(),
            crtc_ids: Vec::new(),
            encoder_ids: Vec::new(),
            connector_ids: Vec::new(),
            object_by_id: HashMap::new(),
            id_allocator: SparseIdAlloc::new(1, u32::MAX),
        }
    }
}

impl DrmKmsObjectStore {
    pub fn alloc_object_id(&mut self) -> Result<KmsObjectId> {
        let id = self
            .id_allocator
            .alloc()
            .ok_or_else(|| Error::with_message(Errno::ENOSPC, "all KMS object IDs are in use"))?;

        Ok(id)
    }

    pub fn collect_object_ids(&self, type_: DrmKmsObjectType) -> Vec<KmsObjectId> {
        let res: &[KmsObjectId] = match type_ {
            DrmKmsObjectType::Crtc => &self.crtc_ids,
            DrmKmsObjectType::Connector => &self.connector_ids,
            DrmKmsObjectType::Encoder => &self.encoder_ids,
            DrmKmsObjectType::Plane => &self.plane_ids,
            _ => &[],
        };
        res.to_vec()
    }

    pub fn add_object(&mut self, object: DrmKmsObject) -> Result<KmsObjectId> {
        let id = self.alloc_object_id()?;

        match &object {
            DrmKmsObject::Plane(_) => self.plane_ids.push(id),
            DrmKmsObject::Crtc(_) => self.crtc_ids.push(id),
            DrmKmsObject::Encoder(_) => self.encoder_ids.push(id),
            DrmKmsObject::Connector(_) => self.connector_ids.push(id),
        }

        self.object_by_id.insert(id, object);
        Ok(id)
    }

    pub fn get_object_id_from_index(
        &self,
        index: KmsObjectIndex,
        type_: DrmKmsObjectType,
    ) -> Option<KmsObjectId> {
        match type_ {
            DrmKmsObjectType::Crtc => self.crtc_ids.get(index).copied(),
            DrmKmsObjectType::Connector => self.connector_ids.get(index).copied(),
            DrmKmsObjectType::Encoder => self.encoder_ids.get(index).copied(),
            DrmKmsObjectType::Plane => self.plane_ids.get(index).copied(),
            _ => None,
        }
    }

    pub fn get_object_index(
        &self,
        id: KmsObjectId,
        type_: DrmKmsObjectType,
    ) -> Option<KmsObjectIndex> {
        let object = self.object_by_id.get(&id)?;
        let object_ids = match (type_, object) {
            (DrmKmsObjectType::Any | DrmKmsObjectType::Plane, DrmKmsObject::Plane(_)) => {
                &self.plane_ids
            }
            (DrmKmsObjectType::Any | DrmKmsObjectType::Crtc, DrmKmsObject::Crtc(_)) => {
                &self.crtc_ids
            }
            (DrmKmsObjectType::Any | DrmKmsObjectType::Encoder, DrmKmsObject::Encoder(_)) => {
                &self.encoder_ids
            }
            (DrmKmsObjectType::Any | DrmKmsObjectType::Connector, DrmKmsObject::Connector(_)) => {
                &self.connector_ids
            }
            _ => return None,
        };

        object_ids.iter().position(|object_id| *object_id == id)
    }

    pub fn lookup_object(&self, id: KmsObjectId) -> Option<&DrmKmsObject> {
        self.object_by_id.get(&id)
    }

    pub fn lookup_crtc(&self, id: KmsObjectId) -> Option<&DrmCrtc> {
        match self.lookup_object(id)? {
            DrmKmsObject::Crtc(crtc) => Some(crtc),
            _ => None,
        }
    }

    pub fn lookup_plane(&self, id: KmsObjectId) -> Option<&DrmPlane> {
        match self.lookup_object(id)? {
            DrmKmsObject::Plane(plane) => Some(plane),
            _ => None,
        }
    }

    pub fn lookup_encoder(&self, id: KmsObjectId) -> Option<&DrmEncoder> {
        match self.lookup_object(id)? {
            DrmKmsObject::Encoder(encoder) => Some(encoder),
            _ => None,
        }
    }

    pub fn lookup_connector(&self, id: KmsObjectId) -> Option<&DrmConnector> {
        match self.lookup_object(id)? {
            DrmKmsObject::Connector(connector) => Some(connector),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum DrmKmsObject {
    Plane(DrmPlane),
    Crtc(DrmCrtc),
    Encoder(DrmEncoder),
    Connector(DrmConnector),
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrmKmsObjectType {
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

impl TryFrom<u32> for DrmKmsObjectType {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Any),
            0xCCCC_CCCC => Ok(Self::Crtc),
            0xC0C0_C0C0 => Ok(Self::Connector),
            0xE0E0_E0E0 => Ok(Self::Encoder),
            0xDEDE_DEDE => Ok(Self::Mode),
            0xB0B0_B0B0 => Ok(Self::Property),
            0xFBFB_FBFB => Ok(Self::Framebuffer),
            0xBBBB_BBBB => Ok(Self::Blob),
            0xEEEE_EEEE => Ok(Self::Plane),
            _ => return_errno_with_message!(Errno::EINVAL, "the DRM KMS object type is invalid"),
        }
    }
}
