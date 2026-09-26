// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};

use aster_core::prelude::*;
use ostd::mm::VmIo;
use ostd_pod::Pod;

use super::ioctl_defs::*;
use crate::{
    file::{DrmClientCaps, DrmFile},
    kms::objects::{
        DrmKmsObject, DrmKmsObjectStore, DrmKmsObjectType, KmsObjectId,
        plane::DrmPlaneType,
        property::{DRM_PROP_NAME_LEN, DrmPropertyAttachments, DrmPropertyFlags, DrmPropertyKind},
    },
    utils::DrmModeModeInfo,
};

impl DrmFile {
    pub(super) fn drm_mode_get_resources(&self, cmd: DrmIoctlModeGetResources) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetResources = args_ptr.read()?;
            let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;

            let (crtc_ids, encoder_ids, connector_ids, framebuffer_ids) = {
                let object_store = mode_config.object_store().lock();

                (
                    object_store.collect_object_ids(DrmKmsObjectType::Crtc),
                    object_store.collect_object_ids(DrmKmsObjectType::Encoder),
                    object_store.collect_object_ids(DrmKmsObjectType::Connector),
                    // TODO: Return registered framebuffer IDs after framebuffer objects are tracked by
                    // `DrmModeConfig`; this list is currently always empty.
                    object_store.collect_object_ids(DrmKmsObjectType::Framebuffer),
                )
            };

            copy_array_to_user(args_ptr.vm(), args.crtc_id_ptr, args.count_crtcs, &crtc_ids)?;
            copy_array_to_user(
                args_ptr.vm(),
                args.encoder_id_ptr,
                args.count_encoders,
                &encoder_ids,
            )?;
            copy_array_to_user(
                args_ptr.vm(),
                args.connector_id_ptr,
                args.count_connectors,
                &connector_ids,
            )?;
            copy_array_to_user(
                args_ptr.vm(),
                args.fb_id_ptr,
                args.count_fbs,
                &framebuffer_ids,
            )?;

            args.count_crtcs = drm_array_len(&crtc_ids)?;
            args.count_encoders = drm_array_len(&encoder_ids)?;
            args.count_connectors = drm_array_len(&connector_ids)?;
            args.count_fbs = drm_array_len(&framebuffer_ids)?;

            let min_size = mode_config.min_fb_size();
            let max_size = mode_config.max_fb_size();
            args.min_width = min_size.width();
            args.max_width = max_size.width();
            args.min_height = min_size.height();
            args.max_height = max_size.height();

            args_ptr.write(&args)?;

            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_get_crtc(&self, cmd: DrmIoctlModeGetCrtc) -> Result<i32> {
        let mut args: DrmModeCrtc = cmd.read()?;
        let (gamma_size, x, y, fb_id, mode) = {
            let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;
            let object_store = mode_config.object_store().lock();

            let crtc = object_store
                .lookup_crtc(args.crtc_id)
                .ok_or(Errno::ENOENT)?;
            let crtc_state_snapshot = crtc.state_snapshot();

            let primary_plane = object_store
                .lookup_plane(crtc.primary_plane_id())
                .ok_or(Errno::ENOENT)?;
            let plane_state_snapshot = primary_plane.state_snapshot();

            (
                crtc.gamma_size(),
                plane_state_snapshot.crtc_rect().x(),
                plane_state_snapshot.crtc_rect().y(),
                plane_state_snapshot.fb_id().unwrap_or(0),
                crtc_state_snapshot
                    .is_enabled()
                    .then(|| crtc_state_snapshot.display_mode())
                    .flatten(),
            )
        };

        args.gamma_size = gamma_size;
        args.x = x;
        args.y = y;
        args.fb_id = fb_id;
        if let Some(mode) = mode {
            args.mode = mode.into();
            args.mode_valid = 1;
        } else {
            args.mode_valid = 0;
        }

        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_mode_get_encoder(&self, cmd: DrmIoctlModeGetEncoder) -> Result<i32> {
        let mut args: DrmModeGetEncoder = cmd.read()?;
        let (crtc_id, encoder_type, possible_crtcs, possible_clones) = {
            let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;
            let object_store = mode_config.object_store().lock();

            let encoder = object_store
                .lookup_encoder(args.encoder_id)
                .ok_or(Errno::ENOENT)?;

            (
                encoder.current_crtc_id().unwrap_or(0),
                encoder.type_() as u32,
                encoder.possible_crtcs().to_vec(),
                encoder.possible_clones().to_vec(),
            )
        };

        let mut possible_crtcs_mask = 0;
        for index in possible_crtcs {
            possible_crtcs_mask |= 1u32 << index;
        }

        let mut possible_clones_mask = 0;
        for index in possible_clones {
            possible_clones_mask |= 1u32 << index;
        }

        args.crtc_id = crtc_id;
        args.encoder_type = encoder_type;
        args.possible_crtcs = possible_crtcs_mask;
        args.possible_clones = possible_clones_mask;

        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_mode_get_connector(&self, cmd: DrmIoctlModeGetConnector) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetConnector = args_ptr.read()?;

            // Linux treats `GETCONNECTOR` with `count_modes == 0` as a forced probe
            // request. Only the DRM master is allowed to refresh the connector
            // state in this path; non-master callers fall back to a read-only query.
            if args.count_modes == 0 && self.is_master() {
                self.kms_device()
                    .ok_or(Errno::EINVAL)?
                    .probe_connector(args.connector_id)?;
            }

            let (
                mode_info,
                prop_ids,
                prop_values,
                encoder_ids,
                encoder_id,
                connector_type,
                connector_type_id,
                connection,
                mm_width,
                mm_height,
                subpixel,
            ) = {
                let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;
                let object_store = mode_config.object_store().lock();

                let connector = object_store
                    .lookup_connector(args.connector_id)
                    .ok_or(Errno::ENOENT)?;
                let state_snapshot = connector.state_snapshot();
                let probe_state_snapshot = connector.probe_state_snapshot();

                let mut encoder_ids: Vec<KmsObjectId> = Vec::new();
                for index in connector.possible_encoders() {
                    encoder_ids.push(
                        object_store
                            .get_object_id_from_index(*index, DrmKmsObjectType::Encoder)
                            .ok_or(Errno::ENOENT)?,
                    );
                }

                let (prop_ids, prop_values) =
                    self.visible_property_values(&object_store, connector.properties())?;

                let mode_info: Vec<DrmModeModeInfo> = probe_state_snapshot
                    .display_modes()
                    .iter()
                    .copied()
                    .map(Into::into)
                    .collect();
                let display_info = probe_state_snapshot.display_info();

                (
                    mode_info,
                    prop_ids,
                    prop_values,
                    encoder_ids,
                    state_snapshot.encoder_id().unwrap_or(0),
                    connector.type_() as u32,
                    connector.type_index(),
                    probe_state_snapshot.status() as u32,
                    display_info.mm_width(),
                    display_info.mm_height(),
                    display_info.subpixel_order(),
                )
            };

            copy_array_to_user(args_ptr.vm(), args.modes_ptr, args.count_modes, &mode_info)?;
            copy_array_to_user(args_ptr.vm(), args.props_ptr, args.count_props, &prop_ids)?;
            copy_array_to_user(
                args_ptr.vm(),
                args.prop_values_ptr,
                args.count_props,
                &prop_values,
            )?;
            copy_array_to_user(
                args_ptr.vm(),
                args.encoders_ptr,
                args.count_encoders,
                &encoder_ids,
            )?;

            args.count_encoders = drm_array_len(&encoder_ids)?;
            args.count_modes = drm_array_len(&mode_info)?;
            args.count_props = drm_array_len(&prop_ids)?;

            args.encoder_id = encoder_id;
            args.connector_type = connector_type;
            args.connector_type_id = connector_type_id;
            args.connection = connection;
            args.mm_width = mm_width;
            args.mm_height = mm_height;
            args.subpixel = subpixel;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_get_property(&self, cmd: DrmIoctlModeGetProperty) -> Result<i32> {
        let mut args: DrmModeGetProperty = cmd.read()?;

        cmd.with_data_ptr(|args_ptr| {
            let (property_name, property_flags, values, enum_entries) = {
                let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;
                let object_store = mode_config.object_store().lock();

                let property = object_store
                    .lookup_property(args.prop_id)
                    .ok_or(Errno::ENOENT)?;

                let values = match property.kind() {
                    DrmPropertyKind::Range { min, max } => vec![*min, *max],
                    DrmPropertyKind::SignedRange { min, max } => {
                        vec![*min as u64, *max as u64]
                    }
                    DrmPropertyKind::Enum(entries) | DrmPropertyKind::Bitmask(entries) => {
                        entries.iter().map(|entry| entry.value()).collect()
                    }
                    DrmPropertyKind::Object(object_type) => vec![*object_type as u64],
                    DrmPropertyKind::Plain | DrmPropertyKind::Blob => vec![],
                };
                let enum_entries = match property.kind() {
                    DrmPropertyKind::Enum(entries) | DrmPropertyKind::Bitmask(entries) => {
                        entries.to_vec()
                    }
                    _ => Vec::new(),
                };

                (
                    property.name_to_u8(),
                    property.flags().bits(),
                    values,
                    enum_entries,
                )
            };

            copy_array_to_user(args_ptr.vm(), args.values_ptr, args.count_values, &values)?;
            copy_array_to_user(
                args_ptr.vm(),
                args.enum_blob_ptr,
                args.count_enum_blobs,
                &enum_entries,
            )?;

            args.flags = property_flags;
            args.name = property_name;
            args.count_values = values.len() as u32;
            args.count_enum_blobs = enum_entries.len() as u32;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_get_blob(&self, cmd: DrmIoctlModeGetPropBlob) -> Result<i32> {
        let mut args: DrmModeGetBlob = cmd.read()?;

        cmd.with_data_ptr(|args_ptr| {
            let blob = {
                let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;
                let object_store = mode_config.object_store().lock();

                object_store
                    .lookup_property_blob(args.blob_id)
                    .ok_or(Errno::ENOENT)?
                    .clone()
            };

            let data = blob.data();
            if args.data != 0 && args.length != 0 && !data.is_empty() {
                let write_len = core::cmp::min(args.length as usize, data.len());
                args_ptr
                    .vm()
                    .write_bytes(args.data as usize, &data[..write_len])?;
            }

            args.length = blob.len() as u32;
            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_get_plane_resources(
        &self,
        cmd: DrmIoctlModeGetPlaneResources,
    ) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetPlaneRes = args_ptr.read()?;
            let plane_ids = {
                let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;
                let object_store = mode_config.object_store().lock();
                let mut plane_ids = object_store.collect_object_ids(DrmKmsObjectType::Plane);

                if !self.has_client_caps(DrmClientCaps::UNIVERSAL_PLANES) {
                    plane_ids.retain(|plane_id| {
                        object_store
                            .lookup_plane(*plane_id)
                            .is_some_and(|plane| plane.type_() == DrmPlaneType::Overlay)
                    });
                }

                plane_ids
            };

            copy_array_to_user(
                args_ptr.vm(),
                args.plane_id_ptr,
                args.count_planes,
                &plane_ids,
            )?;

            args.count_planes = drm_array_len(&plane_ids)?;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_get_plane(&self, cmd: DrmIoctlModeGetPlane) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetPlane = args_ptr.read()?;
            let (crtc_id, fb_id, possible_crtcs, format_types) = {
                let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;
                let object_store = mode_config.object_store().lock();
                let plane = object_store
                    .lookup_plane(args.plane_id)
                    .ok_or(Errno::ENOENT)?;
                let snapshot = plane.state_snapshot();
                let format_types: Vec<u32> = plane
                    .format_types()
                    .iter()
                    .copied()
                    .map(|f| f as u32)
                    .collect();

                (
                    snapshot.crtc_id().unwrap_or(0),
                    snapshot.fb_id().unwrap_or(0),
                    plane.possible_crtcs().to_vec(),
                    format_types,
                )
            };

            copy_array_to_user(
                args_ptr.vm(),
                args.format_type_ptr,
                args.count_format_types,
                &format_types,
            )?;

            let mut possible_crtcs_mask = 0;
            for index in possible_crtcs {
                possible_crtcs_mask |= 1u32 << index;
            }

            args.crtc_id = crtc_id;
            args.fb_id = fb_id;
            args.possible_crtcs = possible_crtcs_mask;
            // `drm_mode_get_plane::gamma_size` is unused by Linux and must remain zero.
            args.gamma_size = 0;
            args.count_format_types = drm_array_len(&format_types)?;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_object_get_props(&self, cmd: DrmIoctlModeObjectGetProps) -> Result<i32> {
        let mut args: DrmModeObjectGetProps = cmd.read()?;

        cmd.with_data_ptr(|args_ptr| {
            let (prop_ids, prop_values) = {
                let mode_config = self.mode_config().ok_or(Errno::EINVAL)?;
                let object_store = mode_config.object_store().lock();

                let object_type =
                    DrmKmsObjectType::try_from(args.obj_type).map_err(|_| Errno::EINVAL)?;

                let object = object_store
                    .lookup_object(args.obj_id)
                    .ok_or(Errno::ENOENT)?;
                let properties = match object {
                    DrmKmsObject::Plane(plane)
                        if matches!(
                            object_type,
                            DrmKmsObjectType::Any | DrmKmsObjectType::Plane
                        ) =>
                    {
                        plane.properties()
                    }
                    DrmKmsObject::Crtc(crtc)
                        if matches!(
                            object_type,
                            DrmKmsObjectType::Any | DrmKmsObjectType::Crtc
                        ) =>
                    {
                        crtc.properties()
                    }
                    DrmKmsObject::Connector(connector)
                        if matches!(
                            object_type,
                            DrmKmsObjectType::Any | DrmKmsObjectType::Connector
                        ) =>
                    {
                        connector.properties()
                    }
                    _ => return_errno_with_message!(
                        Errno::EINVAL,
                        "the DRM object type does not match the object ID"
                    ),
                };

                self.visible_property_values(&object_store, properties)?
            };

            copy_array_to_user(args_ptr.vm(), args.props_ptr, args.count_props, &prop_ids)?;
            copy_array_to_user(
                args_ptr.vm(),
                args.prop_values_ptr,
                args.count_props,
                &prop_values,
            )?;

            args.count_props = drm_array_len(&prop_ids)?;
            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    fn visible_property_values(
        &self,
        objects: &DrmKmsObjectStore,
        properties: &DrmPropertyAttachments,
    ) -> Result<(Vec<u32>, Vec<u64>)> {
        let mut prop_ids = Vec::new();
        let mut prop_values = Vec::new();

        for entry in properties.entries() {
            let id = entry.property_id();
            let value = entry.initial_value();

            let property = objects.lookup_property(id).ok_or(Errno::ENOENT)?;
            if property.flags().contains(DrmPropertyFlags::ATOMIC)
                && !self.has_client_caps(DrmClientCaps::ATOMIC)
            {
                continue;
            }

            prop_ids.push(id);
            prop_values.push(value);
        }

        Ok((prop_ids, prop_values))
    }
}

fn drm_array_len<T>(values: &[T]) -> Result<u32> {
    u32::try_from(values.len())
        .map_err(|_| Error::with_message(Errno::EOVERFLOW, "the DRM array is too large"))
}

fn copy_array_to_user<T: Pod>(
    userspace: &impl VmIo,
    user_ptr: u64,
    user_capacity: u32,
    values: &[T],
) -> Result<()> {
    let copy_count = core::cmp::min(user_capacity as usize, values.len());
    if copy_count == 0 {
        return Ok(());
    }
    if user_ptr == 0 {
        return_errno_with_message!(Errno::EFAULT, "the DRM userspace array pointer is null");
    }

    let user_addr = usize::try_from(user_ptr).map_err(|_| {
        Error::with_message(
            Errno::EFAULT,
            "the DRM userspace array pointer is out of range",
        )
    })?;
    userspace.write_slice(user_addr, &values[..copy_count])?;
    Ok(())
}

/// `struct drm_mode_card_res` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L262-L275>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeGetResources {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,

    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,

    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

/// `struct drm_mode_crtc` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L277-L290>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeCrtc {
    set_connectors_ptr: u64,
    count_connectors: u32,

    crtc_id: u32,
    fb_id: u32,

    x: u32,
    y: u32,

    gamma_size: u32,
    mode_valid: u32,
    mode: DrmModeModeInfo,
}

/// `struct drm_mode_get_encoder` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L375-L383>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeGetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

/// `struct drm_mode_get_connector` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L425-L516>.
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeGetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,

    count_modes: u32,
    count_props: u32,
    count_encoders: u32,

    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
}

/// `struct drm_mode_get_property` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L559-L616>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeGetProperty {
    values_ptr: u64,
    enum_blob_ptr: u64,

    prop_id: u32,
    flags: u32,
    name: [u8; DRM_PROP_NAME_LEN],

    count_values: u32,
    count_enum_blobs: u32,
}

/// `struct drm_mode_get_blob` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L649-L653>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeGetBlob {
    blob_id: u32,
    length: u32,
    data: u64,
}

/// `struct drm_mode_get_plane_res` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L360-L363>.
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeGetPlaneRes {
    plane_id_ptr: u64,
    count_planes: u32,
}

/// `struct drm_mode_get_plane` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L315-L358>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeGetPlane {
    plane_id: u32,
    crtc_id: u32,
    fb_id: u32,
    possible_crtcs: u32,
    gamma_size: u32,
    count_format_types: u32,
    format_type_ptr: u64,
}

/// `struct drm_mode_obj_get_properties` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L634-L640>.
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeObjectGetProps {
    props_ptr: u64,
    prop_values_ptr: u64,
    count_props: u32,
    obj_id: u32,
    obj_type: u32,
}
