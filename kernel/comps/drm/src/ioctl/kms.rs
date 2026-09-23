// SPDX-License-Identifier: MPL-2.0

use alloc::{
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};

use aster_core::prelude::*;
use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;
use ostd_pod::Pod;

use crate::{
    device::DrmFeatures,
    file::{DrmClientCaps, DrmFile},
    ioctl::ioctl_defs,
    kms::{
        display_mode::{DrmDisplayMode, DrmModeInfo},
        objects::{
            DrmKmsObjectType, KmsObjectIndex,
            connector::DrmConnectorState,
            crtc::DrmCrtcState,
            framebuffer::{DrmFramebuffer, DrmFramebufferFlags},
            plane::{DrmPlaneState, DrmPlaneType},
            property::{
                DRM_PROP_NAME_LEN, DrmPropertyAttachments, DrmPropertyFlags, DrmPropertyKind,
            },
        },
        pixel_format::DrmPixelFormat,
    },
    utils::{DrmRect, DrmSize},
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
        let device = self.device();

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
                // TODO: Enable this capability when `DrmAtomicOps` and
                // `DrmDevice::as_atomic_ops` are introduced. The presence of the
                // atomic operations should be the sole source of truth for atomic
                // modesetting support.
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
                if !device.has_features(DrmFeatures::CURSOR_HOTSPOT) {
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

            let (crtc_ids, encoder_ids, connector_ids) = {
                let object_store = mode_config.object_store().lock();

                (
                    object_store.collect_object_ids(DrmKmsObjectType::Crtc),
                    object_store.collect_object_ids(DrmKmsObjectType::Encoder),
                    object_store.collect_object_ids(DrmKmsObjectType::Connector),
                )
            };
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

    pub(super) fn drm_mode_get_crtc(&self, cmd: ioctl_defs::ModeGetCrtc) -> Result<i32> {
        let mut args: DrmModeCrtc = cmd.read()?;
        let (gamma_size, x, y, fb_id, mode) = {
            let mode_config = self.device().as_kms_ops().unwrap().mode_config();
            let object_store = mode_config.object_store().lock();

            let crtc = object_store
                .lookup_crtc(args.crtc_id)
                .ok_or(Errno::ENOENT)?;
            let crtc_state_snapshot = crtc.state_snapshot();

            let primary_plane = crtc.primary_plane().upgrade().ok_or(Errno::ENOENT)?;
            let plane_state_snapshot = primary_plane.state_snapshot();

            (
                crtc.gamma_size(),
                // TODO: Represent plane source coordinates in 16.16 fixed-point,
                // matching Linux DRM semantics. `source_rect` currently stores
                // integer pixel coordinates, so no >> 16 conversion is applied here.
                plane_state_snapshot.source_rect().x(),
                plane_state_snapshot.source_rect().y(),
                plane_state_snapshot
                    .framebuffer()
                    .upgrade()
                    .map_or(0, |framebuffer| framebuffer.id()),
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
            args.mode = DrmModeInfo::try_from(mode)?;
            args.mode_valid = 1;
        } else {
            args.mode_valid = 0;
        }

        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_mode_set_crtc(&self, cmd: ioctl_defs::ModeSetCrtc) -> Result<i32> {
        let args: DrmModeCrtc = cmd.read()?;
        let kms_ops = self.device().as_kms_ops().unwrap();

        let display_mode: Option<DrmDisplayMode> = (args.mode_valid != 0)
            .then(|| args.mode.try_into())
            .transpose()?;

        if display_mode.is_none() && args.count_connectors != 0 {
            return_errno!(Errno::EINVAL);
        }

        if display_mode.is_some() && (args.count_connectors == 0 || args.fb_id == 0) {
            return_errno!(Errno::EINVAL);
        }

        let connector_count = kms_ops
            .mode_config()
            .object_store()
            .lock()
            .collect_object_ids(DrmKmsObjectType::Connector)
            .len();
        if args.count_connectors as usize > connector_count {
            return_errno_with_message!(Errno::EINVAL, "too many connectors for SETCRTC");
        }

        let connector_ids = cmd.with_data_ptr(|args_ptr| {
            copy_array_from_user(
                args_ptr.vm(),
                args.set_connectors_ptr,
                args.count_connectors,
            )
        })?;

        // Serialize the whole modeset against framebuffer removal. In
        // particular, RMFB checks plane state while holding this lock, so the
        // framebuffer cannot be removed between validation and state commit.
        let object_store = kms_ops.mode_config().object_store().lock();
        let framebuffer = display_mode
            .is_some()
            .then(|| self.lookup_framebuffer(args.fb_id).ok_or(Errno::ENOENT))
            .transpose()?;
        let (crtc, primary_plane, connector_encoders, source_rect) = {
            let crtc = object_store
                .lookup_crtc(args.crtc_id)
                .cloned()
                .ok_or(Errno::ENOENT)?;
            let primary_plane = crtc.primary_plane().upgrade().ok_or(Errno::ENOENT)?;

            if let Some(display_mode) = display_mode {
                let framebuffer = framebuffer
                    .as_ref()
                    .expect("an enabled CRTC has a framebuffer");
                let source_rect = DrmRect::new(
                    args.x,
                    args.y,
                    display_mode.hdisplay(),
                    display_mode.vdisplay(),
                );
                if !primary_plane
                    .pixel_formats()
                    .contains(&framebuffer.pixel_format())
                {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the DRM framebuffer format is unsupported by the primary plane"
                    );
                }

                let framebuffer_size = framebuffer.size();
                if !DrmRect::new(0, 0, framebuffer_size.width(), framebuffer_size.height())
                    .contains_rect(&source_rect)
                {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the CRTC scanout rectangle exceeds the DRM framebuffer"
                    );
                }

                let crtc_mask = 1u32 << crtc.index().get();
                let connector_encoders = connector_ids
                    .iter()
                    .map(|connector_id| {
                        let connector = object_store
                            .lookup_connector(*connector_id)
                            .cloned()
                            .ok_or(Errno::ENOENT)?;
                        let encoder = (0..u32::BITS)
                            .filter(|encoder_index| {
                                connector.possible_encoders() & (1 << encoder_index) != 0
                            })
                            .find_map(|encoder_index| {
                                let encoder_index = KmsObjectIndex::new(encoder_index);
                                object_store
                                    .get_object_id(encoder_index, DrmKmsObjectType::Encoder)
                                    .and_then(|encoder_id| object_store.lookup_encoder(encoder_id))
                            })
                            .filter(|encoder| encoder.possible_crtcs() & crtc_mask != 0)
                            .cloned()
                            .ok_or_else(|| {
                                Error::with_message(
                                    Errno::EINVAL,
                                    "the connector has no encoder compatible with the CRTC",
                                )
                            })?;
                        Ok((connector, encoder))
                    })
                    .collect::<Result<Vec<_>>>()?;

                (crtc, primary_plane, connector_encoders, source_rect)
            } else {
                (crtc, primary_plane, Vec::new(), DrmRect::default())
            }
        };

        // The driver applies the validated configuration before the core state
        // is published. Keeping the object-store lock prevents RMFB from
        // observing an unreferenced framebuffer during this interval.
        kms_ops.set_crtc(
            &crtc,
            framebuffer.as_deref(),
            source_rect,
            display_mode,
            &connector_encoders,
        )?;

        // The hardware operation succeeded, so commit the matching core state.
        if let Some(display_mode) = display_mode {
            crtc.update_state(DrmCrtcState::new(display_mode));
            primary_plane.update_state(DrmPlaneState::new(
                source_rect,
                DrmRect::new(0, 0, display_mode.hdisplay(), display_mode.vdisplay()),
                Arc::downgrade(framebuffer.as_ref().unwrap()),
                Arc::downgrade(&crtc),
            ));
            for (connector, encoder) in connector_encoders {
                encoder.set_current_crtc(Arc::downgrade(&crtc));
                connector.update_state(DrmConnectorState::new(Arc::downgrade(&encoder)));
            }
        } else {
            crtc.update_state(DrmCrtcState::default());
            primary_plane.update_state(DrmPlaneState::default());

            for encoder_id in object_store.collect_object_ids(DrmKmsObjectType::Encoder) {
                let encoder = object_store
                    .lookup_encoder(encoder_id)
                    .expect("a registered encoder ID must resolve");
                if encoder
                    .current_crtc()
                    .upgrade()
                    .is_some_and(|current_crtc| Arc::ptr_eq(&current_crtc, &crtc))
                {
                    encoder.set_current_crtc(Weak::new());
                }
            }
            for connector_id in object_store.collect_object_ids(DrmKmsObjectType::Connector) {
                let connector = object_store
                    .lookup_connector(connector_id)
                    .expect("a registered connector ID must resolve");
                if connector
                    .state_snapshot()
                    .encoder()
                    .upgrade()
                    .is_some_and(|encoder| encoder.current_crtc().upgrade().is_none())
                {
                    connector.update_state(DrmConnectorState::default());
                }
            }
        }

        Ok(0)
    }

    pub(super) fn drm_mode_get_encoder(&self, cmd: ioctl_defs::ModeGetEncoder) -> Result<i32> {
        let mut args: DrmModeGetEncoder = cmd.read()?;
        let (crtc_id, encoder_type, possible_crtcs, possible_clones) = {
            let mode_config = self.device().as_kms_ops().unwrap().mode_config();
            let object_store = mode_config.object_store().lock();

            let encoder = object_store
                .lookup_encoder(args.encoder_id)
                .ok_or(Errno::ENOENT)?;

            let crtc_id = encoder.current_crtc().upgrade().map_or(0, |crtc| crtc.id());

            (
                crtc_id,
                encoder.type_() as u32,
                encoder.possible_crtcs(),
                encoder.possible_clones(),
            )
        };

        args.crtc_id = crtc_id;
        args.encoder_type = encoder_type;
        args.possible_crtcs = possible_crtcs;
        args.possible_clones = possible_clones;

        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_mode_get_connector(&self, cmd: ioctl_defs::ModeGetConnector) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetConnector = args_ptr.read()?;

            // Linux treats `GETCONNECTOR` with `count_modes == 0` as a forced probe
            // request. Only the DRM master is allowed to refresh the connector
            // state in this path; non-master callers fall back to a read-only query.
            if args.count_modes == 0 && self.is_master() {
                let kms_ops = self.device().as_kms_ops().unwrap();
                let connector = {
                    let object_store = kms_ops.mode_config().object_store().lock();
                    object_store
                        .lookup_connector(args.connector_id)
                        .cloned()
                        .ok_or(Errno::ENOENT)?
                };
                let probe_state = kms_ops.probe_connector(&connector)?;
                connector.update_probe_state(probe_state);
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
                let mode_config = self.device().as_kms_ops().unwrap().mode_config();
                let object_store = mode_config.object_store().lock();

                let connector = object_store
                    .lookup_connector(args.connector_id)
                    .ok_or(Errno::ENOENT)?;
                let state_snapshot = connector.state_snapshot();
                let probe_state_snapshot = connector.probe_state_snapshot();

                let mut possible_encoders = connector.possible_encoders();
                let mut encoder_ids = Vec::with_capacity(possible_encoders.count_ones() as usize);
                while possible_encoders != 0 {
                    let encoder_index = KmsObjectIndex::new(possible_encoders.trailing_zeros());
                    let encoder_id = object_store
                        .get_object_id(encoder_index, DrmKmsObjectType::Encoder)
                        .ok_or(Errno::ENOENT)?;
                    encoder_ids.push(encoder_id);
                    possible_encoders &= possible_encoders - 1;
                }

                let (prop_ids, prop_values) =
                    self.visible_property_values(connector.properties())?;

                let mode_info: Vec<DrmModeInfo> = probe_state_snapshot
                    .display_modes()
                    .iter()
                    .copied()
                    .map(DrmModeInfo::try_from)
                    .collect::<Result<_>>()?;
                let display_info = probe_state_snapshot.display_info();

                (
                    mode_info,
                    prop_ids,
                    prop_values,
                    encoder_ids,
                    state_snapshot
                        .encoder()
                        .upgrade()
                        .map_or(0, |encoder| encoder.id()),
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

    pub(super) fn drm_mode_get_property(&self, cmd: ioctl_defs::ModeGetProperty) -> Result<i32> {
        let mut args: DrmModeGetProperty = cmd.read()?;

        cmd.with_data_ptr(|args_ptr| {
            let (property_name, property_flags, values, enum_entries) = {
                let mode_config = self.device().as_kms_ops().unwrap().mode_config();
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
            args.count_values = drm_array_len(&values)?;
            args.count_enum_blobs = drm_array_len(&enum_entries)?;

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
                .collect_object_ids(DrmKmsObjectType::Plane)
                .into_iter()
                .filter_map(|plane_id| object_store.lookup_plane(plane_id))
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

        let plane_ids = object_store.collect_object_ids(DrmKmsObjectType::Plane);
        for plane_id in plane_ids.iter().copied() {
            let Some(plane) = object_store.lookup_plane(plane_id) else {
                continue;
            };

            if plane
                .state_snapshot()
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

    pub(super) fn drm_mode_dirty_fb(&self, cmd: ioctl_defs::ModeDirtyFb) -> Result<i32> {
        // TODO: Honor dirtyfb flags, color, and clip rectangles. For now,
        // treat every dirtyfb request as a whole-framebuffer refresh.
        let args: DrmModeFbDirtyCmd = cmd.read()?;

        let framebuffer = self.lookup_framebuffer(args.fb_id).ok_or(Errno::ENOENT)?;
        let kms_ops = self.device().as_kms_ops().unwrap();
        let source_rects = {
            let object_store = kms_ops.mode_config().object_store().lock();
            object_store
                .collect_object_ids(DrmKmsObjectType::Plane)
                .into_iter()
                .filter_map(|plane_id| object_store.lookup_plane(plane_id))
                .filter_map(|plane| {
                    let state = plane.state_snapshot();
                    state
                        .framebuffer()
                        .ptr_eq(&Arc::downgrade(&framebuffer))
                        .then_some(state.source_rect())
                })
                .collect::<Vec<_>>()
        };
        kms_ops.dirty_fb(&framebuffer, &source_rects)?;

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

    pub(super) fn drm_mode_get_plane(&self, cmd: ioctl_defs::ModeGetPlane) -> Result<i32> {
        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmModeGetPlane = args_ptr.read()?;
            let (crtc_id, fb_id, possible_crtcs, pixel_format) = {
                let mode_config = self.device().as_kms_ops().unwrap().mode_config();
                let object_store = mode_config.object_store().lock();
                let plane = object_store
                    .lookup_plane(args.plane_id)
                    .ok_or(Errno::ENOENT)?;
                let plane_state_snapshot = plane.state_snapshot();
                let pixel_format: Vec<u32> = plane
                    .pixel_formats()
                    .iter()
                    .copied()
                    .map(|f| f as u32)
                    .collect();

                let crtc_id = plane_state_snapshot
                    .crtc()
                    .upgrade()
                    .map_or(0, |crtc| crtc.id());
                let fb_id = plane_state_snapshot
                    .framebuffer()
                    .upgrade()
                    .map_or(0, |framebuffer| framebuffer.id());
                (crtc_id, fb_id, plane.possible_crtcs(), pixel_format)
            };

            copy_array_to_user(
                args_ptr.vm(),
                args.format_type_ptr,
                args.count_format_types,
                &pixel_format,
            )?;

            args.crtc_id = crtc_id;
            args.fb_id = fb_id;
            args.possible_crtcs = possible_crtcs;
            // `drm_mode_get_plane::gamma_size` is unused by Linux and must remain zero.
            args.gamma_size = 0;
            args.count_format_types = drm_array_len(&pixel_format)?;

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
            let (prop_ids, prop_values) = {
                let mode_config = self.device().as_kms_ops().unwrap().mode_config();
                let object_store = mode_config.object_store().lock();

                let object_type =
                    DrmKmsObjectType::try_from(args.obj_type).map_err(|_| Errno::ENOENT)?;

                let properties = match object_type {
                    DrmKmsObjectType::Plane => object_store
                        .lookup_plane(args.obj_id)
                        .map(|plane| plane.properties()),
                    DrmKmsObjectType::Crtc => object_store
                        .lookup_crtc(args.obj_id)
                        .map(|crtc| crtc.properties()),
                    DrmKmsObjectType::Connector => object_store
                        .lookup_connector(args.obj_id)
                        .map(|connector| connector.properties()),
                    DrmKmsObjectType::Encoder => {
                        if object_store.lookup_encoder(args.obj_id).is_some() {
                            return_errno_with_message!(
                                Errno::EINVAL,
                                "the DRM encoder has no property table"
                            );
                        }
                        None
                    }
                    DrmKmsObjectType::Any => {
                        let properties = object_store
                            .lookup_plane(args.obj_id)
                            .map(|plane| plane.properties())
                            .or_else(|| {
                                object_store
                                    .lookup_crtc(args.obj_id)
                                    .map(|crtc| crtc.properties())
                            })
                            .or_else(|| {
                                object_store
                                    .lookup_connector(args.obj_id)
                                    .map(|connector| connector.properties())
                            });
                        if properties.is_none()
                            && object_store.lookup_encoder(args.obj_id).is_some()
                        {
                            return_errno_with_message!(
                                Errno::EINVAL,
                                "the DRM encoder has no property table"
                            );
                        }
                        properties
                    }
                    _ => None,
                }
                .ok_or_else(|| {
                    Error::with_message(
                        Errno::ENOENT,
                        "the DRM object type does not match the object ID",
                    )
                })?;

                self.visible_property_values(properties)?
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
        properties: &DrmPropertyAttachments,
    ) -> Result<(Vec<u32>, Vec<u64>)> {
        let mut prop_ids = Vec::new();
        let mut prop_values = Vec::new();

        for entry in properties.entries() {
            let property = entry.property().upgrade().ok_or(Errno::ENOENT)?;
            let id = property.id();
            let value = entry.initial_value();

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

    userspace.write_slice(user_ptr as usize, &values[..copy_count])?;
    Ok(())
}

fn copy_array_from_user<T: Pod>(
    userspace: &impl VmIo,
    user_ptr: u64,
    count: u32,
) -> Result<Vec<T>> {
    let count = usize::try_from(count).map_err(|_| Errno::EOVERFLOW)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if user_ptr == 0 {
        return_errno_with_message!(Errno::EFAULT, "the DRM userspace array pointer is null");
    }

    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| Error::with_message(Errno::ENOMEM, "failed to allocate a DRM array"))?;
    values.resize(count, T::new_zeroed());
    userspace.read_slice(user_ptr as usize, values.as_mut_slice())?;

    Ok(values)
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

/// `struct drm_mode_fb_dirty_cmd` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm_mode.h#L744-L777>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct DrmModeFbDirtyCmd {
    fb_id: u32,
    flags: u32,
    color: u32,
    num_clips: u32,
    clips_ptr: u64,
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
