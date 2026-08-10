// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_drm::{DrmDevice, DrmDeviceCapFlags, DrmFeatures};
use ostd::mm::VmIo;

use crate::{
    device::drm::{DrmMinorType, has_current_sys_admin, ioctl::*, minor::DrmMinor},
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::{
        Process,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::RawIoctl,
};

#[derive(Debug, Default)]
struct DrmFileCaps {
    /// True when the client has asked us to expose stereo 3D mode flags.
    has_stereo: AtomicBool,
    /// True if client understands CRTC primary planes and cursor planes
    /// in the plane list. Automatically set when atomic is set.
    has_universal_planes: AtomicBool,
    /// True if client understands atomic properties.
    has_atomic: AtomicBool,
    /// True, if client can handle picture aspect ratios, and has requested
    /// to pass this information along with the mode.
    has_aspect_ratio: AtomicBool,
    /// True if client understands writeback connectors.
    has_writeback_connectors: AtomicBool,
    /// This client is capable of handling the cursor plane with the
    /// restrictions imposed on it by the virtualized drivers.
    has_virtualized_cursor_plane: AtomicBool,
}

#[derive(Debug, Default)]
struct DrmFileAuthState {
    /// Tracks the current owner process for this file's master-management checks.
    ///
    /// For files that have never been master, this owner can follow the current
    /// ioctl caller (e.g., after fd passing). Once the file has been master,
    /// ownership is frozen to preserve "same process can reacquire master"
    /// semantics.
    owner_process_pid: u32,
    /// Indicates whether this file has ever successfully become DRM master.
    ///
    /// This is sticky after the first successful `SET_MASTER` and is used to
    /// gate non-root master reacquisition to the same owner process.
    was_master: bool,
    /// Tracks legacy primary-node authentication state for this file.
    ///
    /// Currently does not implement legacy auth ioctls
    /// (`DRM_IOCTL_GET_MAGIC`/`DRM_IOCTL_AUTH_MAGIC`) to transition this flag.
    /// `is_authenticated()` also treats current master as authenticated.
    authenticated: bool,
}

/// Represents an open DRM file descriptor exposed to userspace.
///
/// `DrmFile` is created on each successful `open()` of a DRM device node
/// (e.g. `/dev/dri/cardX`, `/dev/dri/renderDX`). It serves as the **per-open
/// execution context** for all userspace interactions with the DRM subsystem.
///
/// Responsibilities:
/// - Dispatching ioctl requests issued from userspace.
/// - Enforcing access restrictions and semantics defined by the associated
///   DRM minor (primary, render, control, etc.).
///
/// `DrmFile` does not own device-wide state. Instead, it holds a reference to
/// the `DrmMinor` through which it was opened, and all operations are ultimately
/// routed to the underlying `DrmDevice` shared by all minors of the same device.
///
/// Each `DrmFile` instance is independent and represents a single userspace
/// file descriptor.
///
#[derive(Debug)]
pub(super) struct DrmFile {
    file_id: u32,
    minor: Arc<DrmMinor>,
    caps: DrmFileCaps,
    auth_state: Mutex<DrmFileAuthState>,
}

impl DrmFile {
    pub(super) fn new(file_id: u32, minor: Arc<DrmMinor>) -> Self {
        let owner_process_pid = Process::current().map_or(0, |process| process.pid());
        let is_master = minor.is_master(file_id);

        let auth_state = DrmFileAuthState {
            owner_process_pid,
            was_master: is_master,
            authenticated: is_master,
        };

        Self {
            file_id,
            minor,
            caps: DrmFileCaps::default(),
            auth_state: Mutex::new(auth_state),
        }
    }

    pub(super) fn is_master(&self) -> bool {
        self.minor.is_master(self.file_id)
    }

    pub(super) fn minor_type(&self) -> DrmMinorType {
        self.minor.type_()
    }

    pub(super) fn is_authenticated(&self) -> bool {
        self.is_master() || self.auth_state.lock().authenticated
    }

    pub(super) fn has_feature(&self, feature: DrmFeatures) -> bool {
        self.device().has_feature(feature)
    }

    /// Keep tracking the ioctl caller while this file has never been master,
    /// so fd passing can update ownership. After the file has been master once,
    /// keep owner pid stable to enforce same-owner master reacquisition semantics.
    fn update_owner_process(&self) {
        let mut auth_state = self.auth_state.lock();

        if auth_state.was_master {
            return;
        }

        if let Some(process) = Process::current() {
            auth_state.owner_process_pid = process.pid();
        }
    }

    fn master_check_permission(&self) -> bool {
        if has_current_sys_admin() {
            return true;
        }

        let auth_state = self.auth_state.lock();

        let is_same_process =
            Process::current().is_some_and(|process| process.pid() == auth_state.owner_process_pid);
        let is_previous_master = auth_state.was_master;

        is_previous_master && is_same_process
    }

    fn device(&self) -> &Arc<dyn DrmDevice> {
        self.minor.device()
    }
}

impl Drop for DrmFile {
    fn drop(&mut self) {
        self.minor.drop_master(self.file_id)
    }
}

impl Pollable for DrmFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for DrmFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "drm: read not supported");
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "drm: write not supported");
    }
}

impl PerOpenFileOps for DrmFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        self.update_owner_process();

        dispatch_drm_ioctl!(
            self,
            match raw_ioctl {
                cmd @ DrmIoctlVersion => {
                    let mut args: DrmVersion = cmd.read()?;

                    let dev = self.device();
                    let name = dev.name();
                    let name_len = name.len();
                    let desc = dev.desc();
                    let desc_len = desc.len();
                    // These fields are legacy in modern DRM userspace flows.
                    // Keep reporting them to preserve `DRM_IOCTL_VERSION` ABI compatibility.
                    let date = "0";
                    let date_len = date.len();
                    let major = 0;
                    let minor = 0;
                    let patch_level = 0;

                    cmd.with_data_ptr(|args_ptr| {
                        // Linux `drm_copy_field` semantics:
                        // copy each field independently with truncation,
                        // then always report the full source length.
                        if args.name_len != 0 {
                            let write_len = core::cmp::min(args.name_len, name_len);
                            args_ptr
                                .vm()
                                .write_bytes(args.name, &name.as_bytes()[..write_len])?;
                        }

                        if args.desc_len != 0 {
                            let write_len = core::cmp::min(args.desc_len, desc_len);
                            args_ptr
                                .vm()
                                .write_bytes(args.desc, &desc.as_bytes()[..write_len])?;
                        }

                        if args.date_len != 0 {
                            let write_len = core::cmp::min(args.date_len, date_len);
                            args_ptr
                                .vm()
                                .write_bytes(args.date, &date.as_bytes()[..write_len])?;
                        }

                        args.name_len = name_len;
                        args.desc_len = desc_len;
                        args.date_len = date_len;
                        args.version_major = major;
                        args.version_minor = minor;
                        args.version_patchlevel = patch_level;

                        args_ptr.write(&args)?;
                        Ok(())
                    })?;

                    Ok(0)
                }
                cmd @ DrmIoctlGetCap => {
                    use DrmGetCapability::*;

                    let mut args: DrmGetCap = cmd.read()?;
                    let cap = DrmGetCapability::try_from(args.capability)?;
                    let device = self.device();

                    let value = match cap {
                        TimestampMonotonic => 1,
                        Prime => (DrmPrimeValue::IMPORT | DrmPrimeValue::EXPORT).bits(),
                        SyncObj => self.has_feature(DrmFeatures::SYNCOBJ) as u64,
                        SyncObjTimeline => self.has_feature(DrmFeatures::SYNCOBJ_TIMELINE) as u64,
                        _ => {
                            if !self.has_feature(DrmFeatures::MODESET) {
                                return_errno!(Errno::EOPNOTSUPP);
                            }
                            let flags = device.caps().flags();
                            match cap {
                                DumbBuffer => flags.contains(DrmDeviceCapFlags::DUMB_BUFFER) as u64,
                                VblankHighCrtc => 1,
                                DumbPreferredDepth => device.caps().preferred_color_depth() as u64,
                                DumbPreferShadow => {
                                    flags.contains(DrmDeviceCapFlags::SHADOW_BUFFER) as u64
                                }
                                AsyncPageFlip => {
                                    flags.contains(DrmDeviceCapFlags::ASYNC_PAGE_FLIP) as u64
                                }
                                PageFlipTarget => {
                                    flags.contains(DrmDeviceCapFlags::PAGE_FLIP_TARGET) as u64
                                }
                                CursorWidth => device.caps().cursor_rect().width() as u64,
                                CursorHeight => device.caps().cursor_rect().height() as u64,
                                Addfb2Modifiers => {
                                    flags.contains(DrmDeviceCapFlags::FB_MODIFIERS) as u64
                                }
                                CrtcInVblankEvent => 1,
                                AtomicAsyncPageFlip => {
                                    (self.has_feature(DrmFeatures::ATOMIC)
                                        && flags.contains(DrmDeviceCapFlags::ASYNC_PAGE_FLIP))
                                        as u64
                                }
                                _ => 0,
                            }
                        }
                    };

                    args.value = value;

                    cmd.write(&args)?;
                    Ok(0)
                }
                cmd @ DrmIoctlSetClientCap => {
                    use DrmSetCapability::*;
                    let args: DrmSetClientCap = cmd.read()?;

                    match DrmSetCapability::try_from(args.capability)? {
                        Stereo3D => match args.value {
                            0 | 1 => {
                                self.caps
                                    .has_stereo
                                    .store(args.value == 1, Ordering::Relaxed);
                            }
                            _ => return_errno!(Errno::EINVAL),
                        },
                        UniversalPlane => {
                            match args.value {
                                0 | 1 => {
                                    self.caps
                                        .has_universal_planes
                                        .store(args.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            };
                        }
                        Atomic => {
                            if !self.has_feature(DrmFeatures::ATOMIC) {
                                return_errno!(Errno::EOPNOTSUPP);
                            }

                            match args.value {
                                0..=2 => {
                                    let v = args.value;

                                    self.caps.has_atomic.store(v >= 1, Ordering::Relaxed);
                                    self.caps
                                        .has_universal_planes
                                        .store(v >= 1, Ordering::Relaxed);
                                    self.caps.has_aspect_ratio.store(v == 2, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            }
                        }
                        AspectRatio => {
                            match args.value {
                                0 | 1 => {
                                    self.caps
                                        .has_aspect_ratio
                                        .store(args.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            };
                        }
                        WritebackConnectors => {
                            if !self.caps.has_atomic.load(Ordering::Relaxed) {
                                return_errno!(Errno::EINVAL);
                            }

                            match args.value {
                                0 | 1 => {
                                    self.caps
                                        .has_writeback_connectors
                                        .store(args.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            };
                        }
                        CursorPlaneHostport => {
                            if !self.has_feature(DrmFeatures::CURSOR_HOTSPOT)
                                && self.caps.has_atomic.load(Ordering::Relaxed)
                            {
                                return_errno!(Errno::EOPNOTSUPP);
                            }

                            match args.value {
                                0 | 1 => {
                                    self.caps
                                        .has_virtualized_cursor_plane
                                        .store(args.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            };
                        }
                    }
                    Ok(0)
                }
                DrmIoctlSetMaster => {
                    if !self.master_check_permission() {
                        return_errno!(Errno::EACCES)
                    }

                    self.minor.set_master(self.file_id)?;
                    let mut auth_state = self.auth_state.lock();
                    auth_state.was_master = true;
                    auth_state.authenticated = true;
                    Ok(0)
                }
                DrmIoctlDropMaster => {
                    if !self.master_check_permission() {
                        return_errno!(Errno::EACCES);
                    }
                    if !self.is_master() {
                        return_errno!(Errno::EINVAL);
                    }

                    self.minor.drop_master(self.file_id);
                    Ok(0)
                }
                _ => {
                    ostd::debug!(
                        "the ioctl command {:#x} is unknown for framebuffer devices",
                        raw_ioctl.cmd()
                    );
                    return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown");
                }
            }
        )
    }
}
