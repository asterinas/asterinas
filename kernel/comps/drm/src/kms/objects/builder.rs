// SPDX-License-Identifier: MPL-2.0

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_core::prelude::*;
use ostd::sync::Mutex;
use sparse_id_alloc::SparseIdAlloc;

use crate::kms::{
    objects::{
        DrmStandardProperty, KmsDynamicObjects, KmsObjectIndex, KmsObjectMask, KmsObjectStore,
        KmsStaticObjects, MAX_OBJECTS_PER_TYPE, OwnedKmsObjectId,
        connector::{DrmConnector, DrmConnectorConfig, DrmConnectorType},
        crtc::{DrmCrtc, DrmCrtcConfig},
        encoder::{DrmEncoder, DrmEncoderConfig, DrmEncoderType},
        plane::{DrmPlane, DrmPlaneConfig, DrmPlaneType},
        property::{
            DrmPropertyAttachments, KmsObjectPropValue, blob::DrmPropertyBlob,
            in_formats::DrmInFormats,
        },
    },
    pixel_format::DrmPixelFormat,
};

/// A builder for KMS object stores during driver initialization.
///
/// Declare objects and their supported connections before calling [`Self::build`].
///
/// Typical usage:
///
/// ```rust,no_run
/// let mut builder = KmsObjectStoreBuilder::default();
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
/// Indices passed to the builder must come from the same builder instance.
///
/// Object configuration constraints:
///
/// - Each CRTC must reference its own primary plane.
/// - The primary plane of a CRTC must have type `Primary`.
/// - If a CRTC has a cursor plane, it must have type `Cursor`.
/// - A primary or cursor plane must also be attached to that CRTC
///   through `attach_plane_to_crtc()`.
/// - Encoders may attach to one or more CRTCs.
/// - Connectors may attach to one or more encoders.
#[derive(Debug, Default)]
pub struct KmsObjectStoreBuilder {
    planes: Vec<DrmPlaneConfig>,
    crtcs: Vec<CrtcSpec>,
    encoders: Vec<DrmEncoderConfig>,
    connectors: Vec<DrmConnectorConfig>,
}

impl KmsObjectStoreBuilder {
    pub fn add_plane(
        &mut self,
        type_: DrmPlaneType,
        pixel_formats: Vec<DrmPixelFormat>,
    ) -> Result<KmsObjectIndex> {
        let index = next_object_index(self.planes.len())?;
        let config = DrmPlaneConfig {
            type_,
            possible_crtcs: KmsObjectMask::ZERO,
            in_formats: DrmInFormats::new(pixel_formats.into_boxed_slice()),
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
        if self.crtcs.get(crtc_index.get()).is_none() {
            return Err(errno!(
                EINVAL,
                "the CRTC index does not refer to a declared CRTC"
            ));
        }

        let plane_config = self
            .planes
            .get_mut(plane_index.get())
            .ok_or(Errno::EINVAL)?;
        plane_config.possible_crtcs.set(crtc_index.get(), true);

        Ok(())
    }

    pub fn attach_encoder_to_crtc(
        &mut self,
        encoder_index: KmsObjectIndex,
        crtc_index: KmsObjectIndex,
    ) -> Result<()> {
        if self.crtcs.get(crtc_index.get()).is_none() {
            return Err(errno!(
                EINVAL,
                "the CRTC index does not refer to a declared CRTC"
            ));
        }

        let encoder_config = self
            .encoders
            .get_mut(encoder_index.get())
            .ok_or(Errno::EINVAL)?;
        encoder_config.possible_crtcs.set(crtc_index.get(), true);

        Ok(())
    }

    pub fn attach_connector_to_encoder(
        &mut self,
        connector_index: KmsObjectIndex,
        encoder_index: KmsObjectIndex,
    ) -> Result<()> {
        if self.encoders.get(encoder_index.get()).is_none() {
            return Err(errno!(
                EINVAL,
                "the encoder index does not refer to a declared encoder"
            ));
        }

        let connector_config = self
            .connectors
            .get_mut(connector_index.get())
            .ok_or(Errno::EINVAL)?;
        connector_config
            .possible_encoders
            .set(encoder_index.get(), true);

        Ok(())
    }

    pub fn build(self) -> Result<KmsObjectStore> {
        self.validate_objects()?;

        let Self {
            planes: plane_configs,
            crtcs: crtc_specs,
            encoders: encoder_configs,
            connectors: connector_configs,
        } = self;

        let id_allocator = Arc::new(Mutex::new(SparseIdAlloc::new(1, u32::MAX)));
        let mut static_objects = KmsStaticObjects::default();

        let mut planes = Vec::with_capacity(plane_configs.len());
        for (plane_index, config) in plane_configs.into_iter().enumerate() {
            let id = OwnedKmsObjectId::alloc(&id_allocator)?;
            let properties = build_plane_properties(&id_allocator, &mut static_objects, &config)?;
            let plane = Arc::new(DrmPlane::new(
                id,
                next_object_index(plane_index)?,
                config,
                properties,
            ));
            static_objects.insert_plane(plane.clone());
            planes.push(plane);
        }

        for (crtc_index, spec) in crtc_specs.into_iter().enumerate() {
            let primary_plane = Arc::downgrade(&planes[spec.primary_plane.get()]);
            let cursor_plane = spec
                .cursor_plane
                .map_or_else(Weak::new, |index| Arc::downgrade(&planes[index.get()]));
            let config = DrmCrtcConfig {
                gamma_size: spec.gamma_lut_size,
                primary_plane,
                cursor_plane,
            };

            let id = OwnedKmsObjectId::alloc(&id_allocator)?;
            let properties = build_object_properties(
                &id_allocator,
                &mut static_objects,
                &[
                    (DrmStandardProperty::Active, 0),
                    // A zero `MODE_ID` means that the CRTC has no active mode.
                    (DrmStandardProperty::ModeId, 0),
                ],
            )?;
            let crtc = Arc::new(DrmCrtc::new(
                id,
                next_object_index(crtc_index)?,
                config,
                properties,
            ));
            static_objects.insert_crtc(crtc);
        }

        for (encoder_index, mut config) in encoder_configs.into_iter().enumerate() {
            // An encoder must always be compatible with itself. `add_encoder()` bounds the
            // index to the clone mask's capacity.
            config.possible_clones.set(encoder_index, true);
            let id = OwnedKmsObjectId::alloc(&id_allocator)?;
            let encoder = Arc::new(DrmEncoder::new(
                id,
                next_object_index(encoder_index)?,
                config,
            ));
            static_objects.insert_encoder(encoder);
        }

        for (connector_index, config) in connector_configs.into_iter().enumerate() {
            let id = OwnedKmsObjectId::alloc(&id_allocator)?;
            let properties = build_object_properties(
                &id_allocator,
                &mut static_objects,
                &[(DrmStandardProperty::CrtcId, 0)],
            )?;
            let connector = Arc::new(DrmConnector::new(
                id,
                next_object_index(connector_index)?,
                config,
                properties,
            ));
            static_objects.insert_connector(connector);
        }

        Ok(KmsObjectStore {
            id_allocator,
            static_objects,
            dynamic_objects: Arc::new(Mutex::new(KmsDynamicObjects::default())),
        })
    }

    fn validate_objects(&self) -> Result<()> {
        if self.crtcs.is_empty() {
            return Err(errno!(EINVAL, "the KMS object collection has no CRTCs"));
        }

        let mut used_primary_planes = KmsObjectMask::ZERO;
        for (crtc_index, crtc) in self.crtcs.iter().enumerate() {
            if used_primary_planes.replace(crtc.primary_plane.get(), true) {
                return Err(errno!(
                    EINVAL,
                    "the KMS primary plane is shared by multiple CRTCs"
                ));
            }

            let primary_plane = self
                .planes
                .get(crtc.primary_plane.get())
                .ok_or(Errno::EINVAL)?;

            if primary_plane.type_ != DrmPlaneType::Primary {
                return Err(errno!(
                    EINVAL,
                    "the CRTC primary plane is not a primary plane"
                ));
            }
            if !primary_plane.possible_crtcs[crtc_index] {
                return Err(errno!(
                    EINVAL,
                    "the CRTC primary plane is not attached to the CRTC"
                ));
            }

            if let Some(cursor_plane) = crtc.cursor_plane {
                let cursor_plane = self.planes.get(cursor_plane.get()).ok_or(Errno::EINVAL)?;

                if cursor_plane.type_ != DrmPlaneType::Cursor {
                    return Err(errno!(
                        EINVAL,
                        "the CRTC cursor plane is not a cursor plane"
                    ));
                }
                if !cursor_plane.possible_crtcs[crtc_index] {
                    return Err(errno!(
                        EINVAL,
                        "the CRTC cursor plane is not attached to the CRTC"
                    ));
                }
            }
        }

        Ok(())
    }
}

fn next_object_index(object_count: usize) -> Result<KmsObjectIndex> {
    if object_count >= MAX_OBJECTS_PER_TYPE {
        return Err(errno!(EINVAL, "too many KMS objects of one type"));
    }

    Ok(KmsObjectIndex::new(object_count))
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

fn build_plane_properties(
    id_allocator: &Arc<Mutex<SparseIdAlloc>>,
    static_objects: &mut KmsStaticObjects,
    config: &DrmPlaneConfig,
) -> Result<DrmPropertyAttachments> {
    let id = OwnedKmsObjectId::alloc(id_allocator)?;
    let in_formats = Arc::new(DrmPropertyBlob::new_static(
        id,
        config.in_formats.encode_blob(),
    ));
    let in_formats_blob_id = in_formats.id();
    static_objects.insert_property_blob(in_formats);

    build_object_properties(
        id_allocator,
        static_objects,
        &[
            (DrmStandardProperty::PlaneType, config.type_ as u64),
            (DrmStandardProperty::InFormats, in_formats_blob_id as u64),
            (DrmStandardProperty::SrcX, 0),
            (DrmStandardProperty::SrcY, 0),
            (DrmStandardProperty::SrcW, 0),
            (DrmStandardProperty::SrcH, 0),
            (DrmStandardProperty::CrtcX, 0),
            (DrmStandardProperty::CrtcY, 0),
            (DrmStandardProperty::CrtcW, 0),
            (DrmStandardProperty::CrtcH, 0),
            (DrmStandardProperty::FbId, 0),
            (DrmStandardProperty::CrtcId, 0),
        ],
    )
}

fn build_object_properties(
    id_allocator: &Arc<Mutex<SparseIdAlloc>>,
    static_objects: &mut KmsStaticObjects,
    property_values: &[(DrmStandardProperty, KmsObjectPropValue)],
) -> Result<DrmPropertyAttachments> {
    let mut object_properties = DrmPropertyAttachments::default();

    for &(standard, value) in property_values {
        let property = if let Some(property) = static_objects.lookup_standard_property(standard) {
            property.clone()
        } else {
            let id = OwnedKmsObjectId::alloc(id_allocator)?;
            let property = Arc::new(standard.create_property(id));
            static_objects.insert_standard_property(standard, property.clone());
            property
        };
        object_properties.attach(property, value)?;
    }

    Ok(object_properties)
}
