// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use aster_core::prelude::*;
use hashbrown::HashMap;

use crate::{
    kms::{
        DrmKmsObjectStore,
        objects::{
            DrmKmsObject, DrmKmsObjectType, KmsObjectIndex,
            connector::{DrmConnType, DrmConnector},
            crtc::DrmCrtc,
            encoder::{DrmEncoder, DrmEncoderType},
            plane::{DrmPlane, DrmPlaneType},
        },
    },
    utils::DrmDisplayFormat,
};

/// Collects KMS object topology during driver initialization.
///
/// The builder is an init-only helper.
/// Drivers first declare planes, CRTCs, encoders, and connectors,
/// then attach their static topology,
/// and finally call `build()` to validate the topology
/// and materialize a `DrmModeConfig`.
///
/// Typical usage:
///
/// ```rust,ignore
/// let mut builder = DrmKmsObjectBuilder::default();
///
/// let primary = builder.add_plane(DrmPlaneType::Primary, format_types);
/// let crtc = builder.add_crtc(0, primary, None);
/// let encoder = builder.add_encoder(DrmEncoderType::VIRTUAL);
/// let connector = builder.add_connector(DrmConnType::VIRTUAL);
///
/// builder.plane_attach_crtc(primary, crtc)?;
/// builder.encoder_attach_crtc(encoder, crtc)?;
/// builder.connector_attach_encoder(connector, encoder)?;
///
/// let object_store = builder.build()?;
/// ```
///
/// The builder only records static topology.
/// Dynamic runtime state remains inside each final KMS object state.
/// All typed indices must come from the same builder instance.
///
/// Current topology constraints:
///
/// - Each CRTC must reference one primary plane.
/// - The primary plane of a CRTC must have type `Primary`.
/// - If a CRTC has a cursor plane, it must have type `Cursor`.
/// - A primary or cursor plane must also be attached to that CRTC
///   through `plane_attach_crtc()`.
/// - Encoders may attach to one or more CRTCs.
/// - Connectors may attach to one or more encoders.
/// - All topology validation is deferred until `build()`.
#[derive(Debug, Default)]
pub struct DrmKmsObjectBuilder {
    planes: Vec<PendingPlane>,
    crtcs: Vec<PendingCrtc>,
    encoders: Vec<PendingEncoder>,
    connectors: Vec<PendingConnector>,
}

impl DrmKmsObjectBuilder {
    pub fn add_plane(
        &mut self,
        type_: DrmPlaneType,
        format_types: Vec<DrmDisplayFormat>,
    ) -> KmsObjectIndex {
        let pending = PendingPlane::new(type_, format_types);
        let index = self.planes.len();

        self.planes.push(pending);
        index
    }

    pub fn add_crtc(
        &mut self,
        gamma_size: u32,
        primary_plane: KmsObjectIndex,
        cursor_plane: Option<KmsObjectIndex>,
    ) -> KmsObjectIndex {
        let pending = PendingCrtc::new(gamma_size, primary_plane, cursor_plane);
        let index = self.crtcs.len();
        self.crtcs.push(pending);
        index
    }

    pub fn add_encoder(&mut self, type_: DrmEncoderType) -> KmsObjectIndex {
        let pending = PendingEncoder::new(type_);
        let index = self.encoders.len();
        self.encoders.push(pending);
        index
    }

    pub fn add_connector(&mut self, type_: DrmConnType) -> KmsObjectIndex {
        let pending = PendingConnector::new(type_);
        let index = self.connectors.len();
        self.connectors.push(pending);
        index
    }

    pub fn plane_attach_crtc(
        &mut self,
        plane: KmsObjectIndex,
        crtc_index: KmsObjectIndex,
    ) -> Result<()> {
        if self.crtcs.get(crtc_index).is_none() {
            return_errno!(Errno::EINVAL);
        }

        let pending_plane = self.planes.get_mut(plane).ok_or(Errno::EINVAL)?;
        let attached_crtcs = &mut pending_plane.attached_crtcs;
        if !attached_crtcs.contains(&crtc_index) {
            attached_crtcs.push(crtc_index);
        }

        Ok(())
    }

    pub fn encoder_attach_crtc(
        &mut self,
        encoder_index: KmsObjectIndex,
        crtc_index: KmsObjectIndex,
    ) -> Result<()> {
        if self.crtcs.get(crtc_index).is_none() {
            return_errno!(Errno::EINVAL);
        }

        let pending_encoder = self.encoders.get_mut(encoder_index).ok_or(Errno::EINVAL)?;
        let attached_crtcs = &mut pending_encoder.attached_crtcs;
        if !attached_crtcs.contains(&crtc_index) {
            attached_crtcs.push(crtc_index);
        }

        Ok(())
    }

    pub fn connector_attach_encoder(
        &mut self,
        connector_index: KmsObjectIndex,
        encoder_index: KmsObjectIndex,
    ) -> Result<()> {
        if self.encoders.get(encoder_index).is_none() {
            return_errno!(Errno::EINVAL);
        }

        let pending_connector = self
            .connectors
            .get_mut(connector_index)
            .ok_or(Errno::EINVAL)?;
        let attached_encoders = &mut pending_connector.attached_encoders;
        if !attached_encoders.contains(&encoder_index) {
            attached_encoders.push(encoder_index);
        }

        Ok(())
    }

    pub fn build(self) -> Result<DrmKmsObjectStore> {
        self.validate_topology()?;

        let mut object_store = DrmKmsObjectStore::default();
        let mut next_type_index_by_connector_type = HashMap::<DrmConnType, u32>::new();

        for plane in &self.planes {
            let object = DrmKmsObject::Plane(DrmPlane::new(
                plane.type_,
                plane.format_types.clone(),
                &plane.attached_crtcs,
            ));
            let _ = object_store.add_object(object)?;
        }

        for crtc in &self.crtcs {
            let primary_plane_id = object_store
                .get_object_id_from_index(crtc.primary_plane, DrmKmsObjectType::Plane)
                .ok_or(Errno::EINVAL)?;
            let cursor_plane_id = match crtc.cursor_plane {
                Some(cursor_plane) => Some(
                    object_store
                        .get_object_id_from_index(cursor_plane, DrmKmsObjectType::Plane)
                        .ok_or(Errno::EINVAL)?,
                ),
                None => None,
            };

            let object = DrmKmsObject::Crtc(DrmCrtc::new(
                crtc.gamma_lut_size,
                primary_plane_id,
                cursor_plane_id,
            ));
            let _ = object_store.add_object(object)?;
        }

        for encoder in &self.encoders {
            let object =
                DrmKmsObject::Encoder(DrmEncoder::new(encoder.type_, &encoder.attached_crtcs));
            let _ = object_store.add_object(object)?;
        }

        for connector in &self.connectors {
            let next_type_index = next_type_index_by_connector_type
                .entry(connector.type_)
                .or_insert(0);
            let type_index = *next_type_index;
            *next_type_index = (*next_type_index).checked_add(1).ok_or(Errno::EINVAL)?;

            let object = DrmKmsObject::Connector(DrmConnector::new(
                connector.type_,
                type_index,
                &connector.attached_encoders,
            ));
            let _ = object_store.add_object(object)?;
        }

        Ok(object_store)
    }

    fn validate_topology(&self) -> Result<()> {
        if self.planes.len() > 32
            || self.crtcs.len() > 32
            || self.encoders.len() > 32
            || self.connectors.len() > 32
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "KMS topology exceeds the 32-object mask limit"
            );
        }

        if self.crtcs.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "TODO");
        }

        for (crtc_index, crtc) in self.crtcs.iter().enumerate() {
            let primary_plane = self.planes.get(crtc.primary_plane).ok_or(Errno::EINVAL)?;

            if primary_plane.type_ != DrmPlaneType::Primary {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the CRTC primary plane is not a primary plane"
                );
            }
            if !primary_plane.attached_crtcs.contains(&crtc_index) {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the CRTC primary plane is not attached to the CRTC"
                );
            }

            if let Some(cursor_plane) = crtc.cursor_plane {
                let cursor_plane = self.planes.get(cursor_plane).ok_or(Errno::EINVAL)?;

                if cursor_plane.type_ != DrmPlaneType::Cursor {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the CRTC cursor plane is not a cursor plane"
                    );
                }
                if !cursor_plane.attached_crtcs.contains(&crtc_index) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the CRTC cursor plane is not attached to the CRTC"
                    );
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct PendingPlane {
    type_: DrmPlaneType,
    format_types: Vec<DrmDisplayFormat>,
    attached_crtcs: Vec<KmsObjectIndex>,
}

impl PendingPlane {
    fn new(type_: DrmPlaneType, format_types: Vec<DrmDisplayFormat>) -> Self {
        Self {
            type_,
            format_types,
            attached_crtcs: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct PendingCrtc {
    gamma_lut_size: u32,
    primary_plane: KmsObjectIndex,
    cursor_plane: Option<KmsObjectIndex>,
}

impl PendingCrtc {
    fn new(
        gamma_lut_size: u32,
        primary_plane: KmsObjectIndex,
        cursor_plane: Option<KmsObjectIndex>,
    ) -> Self {
        Self {
            gamma_lut_size,
            primary_plane,
            cursor_plane,
        }
    }
}

#[derive(Debug)]
struct PendingEncoder {
    type_: DrmEncoderType,
    attached_crtcs: Vec<KmsObjectIndex>,
}

impl PendingEncoder {
    fn new(type_: DrmEncoderType) -> Self {
        Self {
            type_,
            attached_crtcs: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct PendingConnector {
    type_: DrmConnType,
    attached_encoders: Vec<KmsObjectIndex>,
}

impl PendingConnector {
    fn new(type_: DrmConnType) -> Self {
        Self {
            type_,
            attached_encoders: Vec::new(),
        }
    }
}
