// SPDX-License-Identifier: MPL-2.0

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_core::prelude::*;

use crate::kms::{
    DrmKmsObjectStore,
    objects::{
        KmsObjectIndex, KmsObjectMask, MAX_OBJECTS_PER_TYPE,
        connector::{DrmConnectorConfig, DrmConnectorType},
        crtc::DrmCrtcConfig,
        encoder::{DrmEncoderConfig, DrmEncoderType},
        plane::{DrmPlaneConfig, DrmPlaneType},
    },
    pixel_format::DrmPixelFormat,
};

/// A builder for KMS object topology during driver initialization.
///
/// The builder is an init-only helper.
/// Drivers first declare planes, CRTCs, encoders, and connectors,
/// then attach their static topology,
/// and finally call `build()` to validate the topology
/// and materialize a [`DrmKmsObjectStore`].
///
/// Typical usage:
///
/// ```rust,no_run
/// let mut builder = DrmKmsObjectStoreBuilder::default();
///
/// let primary = builder
///     .add_plane(DrmPlaneType::Primary, vec![DrmPixelFormat::XRGB8888])
///     .unwrap();
/// let crtc = builder.add_crtc(0, primary, None).unwrap();
/// let encoder = builder.add_encoder(DrmEncoderType::Virtual).unwrap();
/// let connector = builder.add_connector(DrmConnectorType::Virtual).unwrap();
///
/// builder.attach_plane_to_crtc(primary, crtc).unwrap();
/// builder.attach_encoder_to_crtc(encoder, crtc).unwrap();
/// builder
///     .attach_connector_to_encoder(connector, encoder)
///     .unwrap();
///
/// let object_store = builder.build().unwrap();
/// ```
///
/// The builder only records static topology.
/// Dynamic runtime state remains inside each final KMS object state.
/// Construction indices are confined to the builder and must come from the
/// same builder instance.
/// Static `possible_*` topology is represented by masks whose bit positions are
/// per-type object indices.
/// Dynamic runtime state refers to other objects through typed weak references.
///
/// Current topology constraints:
///
/// - Each CRTC must reference one primary plane.
/// - The primary plane of a CRTC must have type `Primary`.
/// - If a CRTC has a cursor plane, it must have type `Cursor`.
/// - A primary or cursor plane must also be attached to that CRTC
///   through `attach_plane_to_crtc()`.
/// - Encoders may attach to one or more CRTCs.
/// - Connectors may attach to one or more encoders.
#[derive(Debug, Default)]
pub struct DrmKmsObjectStoreBuilder {
    planes: Vec<DrmPlaneConfig>,
    crtcs: Vec<CrtcSpec>,
    encoders: Vec<DrmEncoderConfig>,
    connectors: Vec<DrmConnectorConfig>,
}

impl DrmKmsObjectStoreBuilder {
    pub fn add_plane(
        &mut self,
        type_: DrmPlaneType,
        pixel_formats: Vec<DrmPixelFormat>,
    ) -> Result<KmsObjectIndex> {
        let index = next_object_index(self.planes.len())?;
        let config = DrmPlaneConfig {
            type_,
            possible_crtcs: KmsObjectMask::ZERO,
            pixel_formats: pixel_formats.into_boxed_slice(),
        };

        self.planes.push(config);
        Ok(index)
    }

    pub fn add_crtc(
        &mut self,
        gamma_lut_size: u32,
        primary_plane: KmsObjectIndex,
        cursor_plane: Option<KmsObjectIndex>,
    ) -> Result<KmsObjectIndex> {
        let index = next_object_index(self.crtcs.len())?;
        let spec = CrtcSpec::new(gamma_lut_size, primary_plane, cursor_plane);

        self.crtcs.push(spec);
        Ok(index)
    }

    pub fn add_encoder(&mut self, type_: DrmEncoderType) -> Result<KmsObjectIndex> {
        let index = next_object_index(self.encoders.len())?;
        let config = DrmEncoderConfig {
            type_,
            possible_crtcs: KmsObjectMask::ZERO,
            // The encoder's own clone bit is added once all encoder indices are validated.
            possible_clones: KmsObjectMask::ZERO,
        };
        self.encoders.push(config);
        Ok(index)
    }

    pub fn add_connector(&mut self, type_: DrmConnectorType) -> Result<KmsObjectIndex> {
        let index = next_object_index(self.connectors.len())?;
        // Linux assigns connector type IDs starting at one, so userspace names
        // the first connector of each type, for example, `Virtual-1`.
        let type_index = self
            .connectors
            .iter()
            .filter(|connector| connector.type_ == type_)
            .count() as u32
            + 1;
        let config = DrmConnectorConfig {
            type_,
            type_index,
            possible_encoders: KmsObjectMask::ZERO,
        };
        self.connectors.push(config);
        Ok(index)
    }

    pub fn attach_plane_to_crtc(
        &mut self,
        plane_index: KmsObjectIndex,
        crtc_index: KmsObjectIndex,
    ) -> Result<()> {
        if self.crtcs.get(crtc_index.get() as usize).is_none() {
            return_errno!(Errno::EINVAL);
        }

        let plane_config = self
            .planes
            .get_mut(plane_index.get() as usize)
            .ok_or(Errno::EINVAL)?;
        plane_config
            .possible_crtcs
            .set(crtc_index.get() as usize, true);

        Ok(())
    }

    pub fn attach_encoder_to_crtc(
        &mut self,
        encoder_index: KmsObjectIndex,
        crtc_index: KmsObjectIndex,
    ) -> Result<()> {
        if self.crtcs.get(crtc_index.get() as usize).is_none() {
            return_errno!(Errno::EINVAL);
        }

        let encoder_config = self
            .encoders
            .get_mut(encoder_index.get() as usize)
            .ok_or(Errno::EINVAL)?;
        encoder_config
            .possible_crtcs
            .set(crtc_index.get() as usize, true);

        Ok(())
    }

    pub fn attach_connector_to_encoder(
        &mut self,
        connector_index: KmsObjectIndex,
        encoder_index: KmsObjectIndex,
    ) -> Result<()> {
        if self.encoders.get(encoder_index.get() as usize).is_none() {
            return_errno!(Errno::EINVAL);
        }

        let connector_config = self
            .connectors
            .get_mut(connector_index.get() as usize)
            .ok_or(Errno::EINVAL)?;
        connector_config
            .possible_encoders
            .set(encoder_index.get() as usize, true);

        Ok(())
    }

    pub fn build(self) -> Result<DrmKmsObjectStore> {
        self.validate_topology()?;

        let Self {
            planes: plane_configs,
            crtcs: crtc_specs,
            encoders: encoder_configs,
            connectors: connector_configs,
        } = self;

        let mut object_store = DrmKmsObjectStore::default();

        let mut planes = Vec::with_capacity(plane_configs.len());
        for (plane_index, config) in plane_configs.into_iter().enumerate() {
            let plane = object_store.create_plane(next_object_index(plane_index)?, config)?;
            planes.push(plane);
        }

        for (crtc_index, spec) in crtc_specs.into_iter().enumerate() {
            let primary_plane = Arc::downgrade(&planes[spec.primary_plane.get() as usize]);
            let cursor_plane = spec.cursor_plane.map_or_else(Weak::new, |index| {
                Arc::downgrade(&planes[index.get() as usize])
            });
            let config = DrmCrtcConfig {
                gamma_size: spec.gamma_lut_size,
                primary_plane,
                cursor_plane,
            };
            object_store.create_crtc(next_object_index(crtc_index)?, config)?;
        }

        for (encoder_index, mut config) in encoder_configs.into_iter().enumerate() {
            // An encoder must always be compatible with itself. `add_encoder()` bounds the
            // index to the clone mask's capacity.
            config.possible_clones.set(encoder_index, true);
            object_store.create_encoder(next_object_index(encoder_index)?, config)?;
        }

        for (connector_index, config) in connector_configs.into_iter().enumerate() {
            object_store.create_connector(next_object_index(connector_index)?, config)?;
        }

        Ok(object_store)
    }

    fn validate_topology(&self) -> Result<()> {
        if self.crtcs.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the KMS topology has no CRTCs");
        }

        for (crtc_index, crtc) in self.crtcs.iter().enumerate() {
            let primary_plane = self
                .planes
                .get(crtc.primary_plane.get() as usize)
                .ok_or(Errno::EINVAL)?;

            if primary_plane.type_ != DrmPlaneType::Primary {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the CRTC primary plane is not a primary plane"
                );
            }
            if !primary_plane.possible_crtcs[crtc_index] {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the CRTC primary plane is not attached to the CRTC"
                );
            }

            if let Some(cursor_plane) = crtc.cursor_plane {
                let cursor_plane = self
                    .planes
                    .get(cursor_plane.get() as usize)
                    .ok_or(Errno::EINVAL)?;

                if cursor_plane.type_ != DrmPlaneType::Cursor {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the CRTC cursor plane is not a cursor plane"
                    );
                }
                if !cursor_plane.possible_crtcs[crtc_index] {
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

fn next_object_index(object_count: usize) -> Result<KmsObjectIndex> {
    if object_count >= MAX_OBJECTS_PER_TYPE {
        return_errno_with_message!(Errno::EINVAL, "too many KMS objects of one type");
    }

    Ok(KmsObjectIndex::new(object_count as u32))
}

#[derive(Debug)]
struct CrtcSpec {
    gamma_lut_size: u32,
    primary_plane: KmsObjectIndex,
    cursor_plane: Option<KmsObjectIndex>,
}

impl CrtcSpec {
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
