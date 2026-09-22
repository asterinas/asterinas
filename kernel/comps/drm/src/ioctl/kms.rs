// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec, vec::Vec};

use aster_core::prelude::*;
use aster_util::fixed_str::FixedCStr;
use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;
use ostd_pod::Pod;

use crate::{
    device::DrmFeatures,
    file::{DrmClientCaps, DrmFile},
    ioctl::ioctl_defs,
    kms::{
        display_mode::DrmModeInfo,
        objects::{
            DrmKmsObjectType, KmsObjectIndex,
            framebuffer::{DrmFramebuffer, DrmFramebufferFlags},
            plane::DrmPlaneType,
            property::{
                DRM_PROP_NAME_LEN, DrmPropertyAttachments, DrmPropertyFlags, DrmPropertyValueType,
            },
        },
        pixel_format::DrmPixelFormat,
    },
    utils::DrmSize,
};

impl DrmFile {
    pub(super) fn drm_set_client_cap(&self, cmd: ioctl_defs::SetClientCap) -> Result<i32> {
        /// DRM client capabilities accepted by `DRM_IOCTL_SET_CLIENT_CAP`.
        ///
        /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L791>.
        #[repr(u64)]
        #[derive(Debug, TryFromInt)]
        enum DrmSetCapability {
            Stereo3D = 0x1,
            UniversalPlane = 0x2,
            Atomic = 0x3,
            AspectRatio = 0x4,
            WritebackConnectors = 0x5,
            CursorPlaneHotspot = 0x6,
        }

        let args: DrmSetClientCap = cmd.read()?;

        let Ok(cap) = DrmSetCapability::try_from(args.capability) else {
            return_errno_with_message!(Errno::EINVAL, "the DRM client capability is unknown");
        };

        match cap {
            DrmSetCapability::Stereo3D => self.set_client_caps(
                DrmClientCaps::STEREO_3D,
                parse_boolean_capability(args.value)?,
            ),
            DrmSetCapability::UniversalPlane => self.set_client_caps(
                DrmClientCaps::UNIVERSAL_PLANES,
                parse_boolean_capability(args.value)?,
            ),
            DrmSetCapability::Atomic => {
                // TODO: Support atomic modesetting.
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "the DRM device lacks atomic modesetting"
                );
            }
            DrmSetCapability::AspectRatio => self.set_client_caps(
                DrmClientCaps::ASPECT_RATIO,
                parse_boolean_capability(args.value)?,
            ),
            DrmSetCapability::WritebackConnectors => {
                if !self.has_client_caps(DrmClientCaps::ATOMIC) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the atomic DRM client capability must be enabled before writeback connectors"
                    );
                }

                self.set_client_caps(
                    DrmClientCaps::WRITEBACK_CONNECTORS,
                    parse_boolean_capability(args.value)?,
                );
            }
            DrmSetCapability::CursorPlaneHotspot => {
                if !self.device().has_features(DrmFeatures::CURSOR_HOTSPOT) {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the DRM device lacks cursor hotspot support"
                    );
                }

                if !self.has_client_caps(DrmClientCaps::ATOMIC) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the atomic DRM client capability must be enabled before cursor hotspots"
                    );
                }

                self.set_client_caps(
                    DrmClientCaps::CURSOR_PLANE_HOTSPOT,
                    parse_boolean_capability(args.value)?,
                );
            }
        }
        Ok(0)
    }

    pub(super) fn drm_mode_get_resources(&self, cmd: ioctl_defs::ModeGetResources) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetResources = args_ptr.read()?;
            let mode_config = self.device().as_kms_ops().unwrap().mode_config();

            let object_store = mode_config.object_store().lock();

            let crtc_ids = object_store.collect_sorted_crtc_ids();
            let encoder_ids = object_store.collect_sorted_encoder_ids();
            let connector_ids = object_store.collect_sorted_connector_ids();

            // All store data needed below has been collected into local values.
            // Release the mutex before copying to userspace, which may fault and
            // does not require access to the object store.
            drop(object_store);

            let framebuffer_ids = self.framebuffer_ids();

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

            args.count_crtcs = crtc_ids.len() as u32;
            args.count_encoders = encoder_ids.len() as u32;
            args.count_connectors = connector_ids.len() as u32;
            args.count_fbs = framebuffer_ids.len() as u32;

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

    pub(super) fn drm_mode_get_crtc(&self, cmd: ioctl_defs::ModeGetCrtc) -> Result<i32> {
        let mut args: DrmModeCrtc = cmd.read()?;
        let mode_config = self.device().as_kms_ops().unwrap().mode_config();
        let object_store = mode_config.object_store().lock();

        let crtc = object_store
            .lookup_crtc_by_id(args.crtc_id)
            .ok_or(Errno::ENOENT)?;

        args.gamma_size = crtc.gamma_size();
        let mode = {
            let crtc_state = crtc.state().lock();
            crtc_state
                .is_enabled()
                .then(|| crtc_state.display_mode())
                .flatten()
        };

        let primary_plane = crtc.primary_plane().upgrade().ok_or(Errno::ENOENT)?;
        let plane_state = primary_plane.state().lock();
        // TODO: Convert source coordinates from 16.16 fixed-point once supported.
        args.x = plane_state.source_rect().x();
        args.y = plane_state.source_rect().y();
        args.fb_id = plane_state
            .framebuffer()
            .upgrade()
            .map_or(0, |framebuffer| framebuffer.id());
        drop(plane_state);

        if let Some(mode) = mode {
            args.mode = DrmModeInfo::try_from(mode)?;
            args.mode_valid = 1;
        } else {
            args.mode_valid = 0;
        }

        drop(object_store);

        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_mode_get_encoder(&self, cmd: ioctl_defs::ModeGetEncoder) -> Result<i32> {
        let mut args: DrmModeGetEncoder = cmd.read()?;
        let mode_config = self.device().as_kms_ops().unwrap().mode_config();
        let object_store = mode_config.object_store().lock();

        let encoder = object_store
            .lookup_encoder_by_id(args.encoder_id)
            .ok_or(Errno::ENOENT)?;

        args.crtc_id = encoder.current_crtc().upgrade().map_or(0, |crtc| crtc.id());
        args.encoder_type = encoder.type_() as u32;
        args.possible_crtcs = encoder.possible_crtcs().as_raw_slice()[0];
        args.possible_clones = encoder.possible_clones().as_raw_slice()[0];

        drop(object_store);

        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_mode_get_connector(&self, cmd: ioctl_defs::ModeGetConnector) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetConnector = args_ptr.read()?;

            // Linux treats `GETCONNECTOR` with `count_modes == 0` as a forced probe request.
            // Only the DRM master is allowed to refresh the connector state in this path;
            // non-master callers fall back to a read-only query.
            if args.count_modes == 0 && self.is_master() {
                let kms_ops = self.device().as_kms_ops().unwrap();
                // Keep the connector alive without holding the object-store mutex
                // across the driver callback, which may perform I/O or acquire other locks.
                let connector = {
                    let object_store = kms_ops.mode_config().object_store().lock();
                    object_store
                        .lookup_connector_by_id(args.connector_id)
                        .cloned()
                        .ok_or(Errno::ENOENT)?
                };
                let probe_state = kms_ops.probe_connector(&connector)?;
                connector.update_probe_state(probe_state);
            }

            let mode_config = self.device().as_kms_ops().unwrap().mode_config();
            let object_store = mode_config.object_store().lock();

            let connector = object_store
                .lookup_connector_by_id(args.connector_id)
                .ok_or(Errno::ENOENT)?;
            args.connector_type = connector.type_() as u32;
            args.connector_type_id = connector.type_index();
            let encoder_id = {
                let state = connector.state().lock();
                state.encoder().upgrade().map_or(0, |encoder| encoder.id())
            };
            args.encoder_id = encoder_id;

            let possible_encoders = connector.possible_encoders();
            let mut encoder_ids = Vec::with_capacity(possible_encoders.count_ones());
            for index in possible_encoders.iter_ones() {
                let encoder_index = KmsObjectIndex::new(index);
                let encoder_id = object_store
                    .lookup_encoder_by_index(encoder_index)
                    .map(|encoder| encoder.id())
                    .ok_or(Errno::ENOENT)?;

                encoder_ids.push(encoder_id);
            }

            let (prop_ids, prop_values) = self.visible_property_values(connector.properties());

            let mode_info = {
                let probe_state = connector.probe_state().lock();
                let display_info = probe_state.display_info();
                args.connection = probe_state.status() as u32;
                args.mm_width = display_info.mm_width();
                args.mm_height = display_info.mm_height();
                args.subpixel = display_info.subpixel_order();

                probe_state
                    .display_modes()
                    .iter()
                    .copied()
                    .map(DrmModeInfo::try_from)
                    .collect::<Result<Vec<_>>>()?
            };

            drop(object_store);

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

            args.count_encoders = encoder_ids.len() as u32;
            args.count_modes = mode_info.len() as u32;
            args.count_props = prop_ids.len() as u32;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_get_property(&self, cmd: ioctl_defs::ModeGetProperty) -> Result<i32> {
        let mut args: DrmModeGetProperty = cmd.read()?;

        cmd.with_data_ptr(|args_ptr| {
            let mode_config = self.device().as_kms_ops().unwrap().mode_config();
            let object_store = mode_config.object_store().lock();

            let property = object_store
                .lookup_property(args.prop_id)
                .ok_or(Errno::ENOENT)?;

            let values = match property.value_type() {
                DrmPropertyValueType::Range { min, max } => vec![*min, *max],
                DrmPropertyValueType::SignedRange { min, max } => {
                    vec![*min as u64, *max as u64]
                }
                DrmPropertyValueType::Enum(entries) | DrmPropertyValueType::Bitmask(entries) => {
                    entries.iter().map(|entry| entry.value()).collect()
                }
                DrmPropertyValueType::Object(object_type) => vec![*object_type as u64],
                DrmPropertyValueType::Plain | DrmPropertyValueType::Blob => vec![],
            };
            let enum_entries = match property.value_type() {
                DrmPropertyValueType::Enum(entries) | DrmPropertyValueType::Bitmask(entries) => {
                    entries.to_vec()
                }
                _ => Vec::new(),
            };

            args.name = *property.name();
            args.flags = property.flags().bits();
            drop(object_store);

            copy_array_to_user(args_ptr.vm(), args.values_ptr, args.count_values, &values)?;
            copy_array_to_user(
                args_ptr.vm(),
                args.enum_blob_ptr,
                args.count_enum_blobs,
                &enum_entries,
            )?;

            args.count_values = values.len() as u32;
            args.count_enum_blobs = enum_entries.len() as u32;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_get_blob(&self, cmd: ioctl_defs::ModeGetPropBlob) -> Result<i32> {
        let mut args: DrmModeGetBlob = cmd.read()?;

        cmd.with_data_ptr(|args_ptr| {
            let blob = {
                let mode_config = self.device().as_kms_ops().unwrap().mode_config();
                let object_store = mode_config.object_store().lock();
                object_store
                    .lookup_property_blob(args.blob_id)
                    .ok_or(Errno::ENOENT)?
                    .clone()
            };

            let data = blob.data();
            if args.length != 0 && !data.is_empty() {
                if args.data == 0 {
                    return_errno_with_message!(
                        Errno::EFAULT,
                        "the DRM blob output pointer is null"
                    );
                }

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

    pub(super) fn drm_mode_add_fb(&self, cmd: ioctl_defs::ModeAddFb) -> Result<i32> {
        let mut args: DrmModeFbCmd = cmd.read()?;

        let pixel_format = DrmPixelFormat::try_from((args.bpp, args.depth))?;

        let mode_config = self.device().as_kms_ops().unwrap().mode_config();
        let min_size = mode_config.min_fb_size();
        let max_size = mode_config.max_fb_size();
        let size = DrmSize::new(args.width, args.height);

        if !size.is_within(
            min_size.width()..=max_size.width(),
            min_size.height()..=max_size.height(),
        ) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the DRM framebuffer dimensions are outside the supported range"
            );
        }

        let supports_pixel_format = {
            let object_store = mode_config.object_store().lock();
            object_store
                .planes()
                .any(|plane| plane.pixel_formats().contains(&pixel_format))
        };
        if !supports_pixel_format {
            return_errno_with_message!(
                Errno::EINVAL,
                "no DRM plane supports the framebuffer pixel format"
            );
        }

        let gem_object = self.lookup_gem_object(args.handle)?;
        let framebuffer = {
            let mut object_store = mode_config.object_store().lock();
            let fb_id = object_store.alloc_object_id()?;
            let framebuffer = DrmFramebuffer::new(
                fb_id,
                DrmSize::new(args.width, args.height),
                pixel_format,
                DrmFramebufferFlags::empty(),
                args.pitch,
                0,
                0,
                gem_object,
            );

            match framebuffer {
                Ok(framebuffer) => Arc::new(framebuffer),
                Err(error) => {
                    object_store.free_object_id(fb_id);
                    return Err(error);
                }
            }
        };
        let fb_id = framebuffer.id();
        args.fb_id = fb_id;

        if let Err(error) = cmd.write(&args) {
            mode_config.object_store().lock().free_object_id(fb_id);
            return Err(error);
        }
        self.add_framebuffer(framebuffer);

        Ok(0)
    }

    pub(super) fn drm_mode_rm_fb(&self, cmd: ioctl_defs::ModeRmFb) -> Result<i32> {
        let fb_id: u32 = cmd.read()?;

        let framebuffer = self.lookup_framebuffer(fb_id).ok_or(Errno::ENOENT)?;
        let mode_config = self.device().as_kms_ops().unwrap().mode_config();
        let mut object_store = mode_config.object_store().lock();

        for plane in object_store.planes() {
            if plane
                .state()
                .lock()
                .framebuffer()
                .ptr_eq(&Arc::downgrade(&framebuffer))
            {
                return_errno_with_message!(
                    Errno::EBUSY,
                    "the DRM framebuffer is still in use by a plane"
                );
            }
        }

        self.remove_framebuffer(fb_id).ok_or(Errno::ENOENT)?;
        object_store.free_object_id(fb_id);

        Ok(0)
    }

    pub(super) fn drm_mode_get_plane_resources(
        &self,
        cmd: ioctl_defs::ModeGetPlaneResources,
    ) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetPlaneRes = args_ptr.read()?;
            let plane_ids = {
                let mode_config = self.device().as_kms_ops().unwrap().mode_config();
                let object_store = mode_config.object_store().lock();
                let universal_planes = self.has_client_caps(DrmClientCaps::UNIVERSAL_PLANES);
                let mut planes = object_store
                    .planes()
                    .filter(|plane| universal_planes || plane.type_() == DrmPlaneType::Overlay)
                    .collect::<Vec<_>>();
                planes.sort_unstable_by_key(|plane| plane.index());

                planes
                    .into_iter()
                    .map(|plane| plane.id())
                    .collect::<Vec<_>>()
            };

            copy_array_to_user(
                args_ptr.vm(),
                args.plane_id_ptr,
                args.count_planes,
                &plane_ids,
            )?;

            args.count_planes = plane_ids.len() as u32;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_get_plane(&self, cmd: ioctl_defs::ModeGetPlane) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetPlane = args_ptr.read()?;
            let pixel_format = {
                let mode_config = self.device().as_kms_ops().unwrap().mode_config();
                let object_store = mode_config.object_store().lock();
                let plane = object_store
                    .lookup_plane_by_id(args.plane_id)
                    .ok_or(Errno::ENOENT)?;
                args.possible_crtcs = plane.possible_crtcs().as_raw_slice()[0];
                // `drm_mode_get_plane::gamma_size` is unused by Linux and must remain zero.
                args.gamma_size = 0;

                let plane_state = plane.state().lock();
                args.crtc_id = plane_state.crtc().upgrade().map_or(0, |crtc| crtc.id());
                args.fb_id = plane_state
                    .framebuffer()
                    .upgrade()
                    .map_or(0, |framebuffer| framebuffer.id());
                drop(plane_state);

                plane
                    .pixel_formats()
                    .iter()
                    .copied()
                    .map(|format| format as u32)
                    .collect::<Vec<_>>()
            };

            copy_array_to_user(
                args_ptr.vm(),
                args.format_type_ptr,
                args.count_format_types,
                &pixel_format,
            )?;

            args.count_format_types = pixel_format.len() as u32;

            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn drm_mode_object_get_props(
        &self,
        cmd: ioctl_defs::ModeObjectGetProps,
    ) -> Result<i32> {
        let mut args: DrmModeObjectGetProps = cmd.read()?;

        cmd.with_data_ptr(|args_ptr| {
            let mode_config = self.device().as_kms_ops().unwrap().mode_config();
            let object_store = mode_config.object_store().lock();

            let object_type =
                DrmKmsObjectType::try_from(args.obj_type).map_err(|_| Errno::ENOENT)?;

            let properties = match object_type {
                DrmKmsObjectType::Plane => object_store
                    .lookup_plane_by_id(args.obj_id)
                    .map(|plane| plane.properties()),
                DrmKmsObjectType::Crtc => object_store
                    .lookup_crtc_by_id(args.obj_id)
                    .map(|crtc| crtc.properties()),
                DrmKmsObjectType::Connector => object_store
                    .lookup_connector_by_id(args.obj_id)
                    .map(|connector| connector.properties()),
                DrmKmsObjectType::Encoder => {
                    if object_store.lookup_encoder_by_id(args.obj_id).is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the DRM encoder has no property table"
                        );
                    }
                    None
                }
                DrmKmsObjectType::Any => object_store.lookup_any_object_properties(args.obj_id)?,
                _ => None,
            }
            .ok_or_else(|| {
                Error::with_message(
                    Errno::ENOENT,
                    "the DRM object type does not match the object ID",
                )
            })?;

            let (prop_ids, prop_values) = self.visible_property_values(properties);
            drop(object_store);

            copy_array_to_user(args_ptr.vm(), args.props_ptr, args.count_props, &prop_ids)?;
            copy_array_to_user(
                args_ptr.vm(),
                args.prop_values_ptr,
                args.count_props,
                &prop_values,
            )?;

            args.count_props = prop_ids.len() as u32;
            args_ptr.write(&args)?;
            Ok(())
        })?;

        Ok(0)
    }

    fn visible_property_values(&self, properties: &DrmPropertyAttachments) -> (Vec<u32>, Vec<u64>) {
        let mut prop_ids = Vec::new();
        let mut prop_values = Vec::new();

        for attachment in properties.attachments() {
            let property = attachment.property();
            let id = property.id();
            let value = attachment.value();

            if property.flags().contains(DrmPropertyFlags::ATOMIC)
                && !self.has_client_caps(DrmClientCaps::ATOMIC)
            {
                continue;
            }

            prop_ids.push(id);
            prop_values.push(value);
        }

        (prop_ids, prop_values)
    }
}

fn parse_boolean_capability(value: u64) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => return_errno_with_message!(
            Errno::EINVAL,
            "a boolean DRM client capability must be zero or one"
        ),
    }
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

    userspace.write_slice(user_ptr as usize, &values[..copy_count])?;
    Ok(())
}

/// `struct drm_set_client_cap` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L879>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmSetClientCap {
    capability: u64,
    value: u64,
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
    mode: DrmModeInfo,
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
    name: FixedCStr<DRM_PROP_NAME_LEN>,

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

/// `struct drm_mode_fb_cmd` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L655-L664>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmModeFbCmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
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
