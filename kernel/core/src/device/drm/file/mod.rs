// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use aster_drm::device::{DrmDevice, DrmDeviceCapFlags, DrmFeatures, DrmMaster};
use ostd::mm::VmIo;

use crate::{
    device::drm::{
        has_current_sys_admin,
        ioctl::*,
        minor::{DrmMinor, DrmMinorType},
    },
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    prelude::*,
    process::{
        Process,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::RawIoctl,
};

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

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
    /// Magic allocated from this file's retained master context.
    ///
    /// It remains stable across `GET_MAGIC` calls and is cleared if the file moves
    /// to a newly created master context.
    magic: Option<u32>,
    /// Master context retained by this file.
    ///
    /// This may differ from the device's current master after `DROP_MASTER`.
    /// Non-primary files do not have a master context.
    master: Option<Arc<DrmMaster>>,
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
/// Each instance represents one open file description and owns its per-open
/// capabilities and authentication state. Primary files may share an
/// [`Arc<DrmMaster>`] while remaining otherwise independent.
///
#[derive(Debug)]
pub(super) struct DrmFile {
    client_id: u64,
    caps: DrmFileCaps,
    auth_state: Mutex<DrmFileAuthState>,
    /// Authentication state referenced weakly by the master magic table.
    authenticated: Arc<AtomicBool>,

    minor: Arc<DrmMinor>,
    device: Arc<dyn DrmDevice>,
}

impl DrmFile {
    pub(super) fn new(minor: Arc<DrmMinor>) -> Self {
        let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
        let owner_process_pid = Process::current().map_or(0, |process| process.pid());
        let device = minor.device().clone();

        // Primary clients join the current master context. If no master exists,
        // the first primary client creates one and becomes master automatically.
        let (master, is_master) = match minor.type_() {
            DrmMinorType::Primary => {
                let (master, is_master) = device.open_primary_client(client_id);
                (Some(master), is_master)
            }
            _ => (None, false),
        };

        let authenticated = Arc::new(AtomicBool::new(is_master));

        let auth_state = DrmFileAuthState {
            owner_process_pid,
            was_master: is_master,
            magic: None,
            master,
        };

        Self {
            client_id,
            caps: DrmFileCaps::default(),
            auth_state: Mutex::new(auth_state),
            authenticated,
            minor,
            device,
        }
    }

    pub(super) fn minor_type(&self) -> DrmMinorType {
        self.minor.type_()
    }

    pub(super) fn has_feature(&self, feature: DrmFeatures) -> bool {
        self.device.has_feature(feature)
    }

    pub(super) fn is_current_master(&self) -> bool {
        self.device.is_current_master(self.client_id)
    }

    pub(super) fn is_authenticated(&self) -> bool {
        self.is_current_master() || self.authenticated.load(Ordering::Acquire)
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

    /// Checks permission to change DRM master ownership.
    ///
    /// `CAP_SYS_ADMIN` may always perform the operation. Otherwise, only the same
    /// process acting through a file that is or was master is permitted, preserving
    /// master control across logind-style file descriptor passing.
    fn check_master_control_permission(&self) -> Result<()> {
        if has_current_sys_admin() {
            return Ok(());
        }

        let auth_state = self.auth_state.lock();
        let is_owner_process =
            Process::current().is_some_and(|process| process.pid() == auth_state.owner_process_pid);

        if auth_state.was_master && is_owner_process {
            Ok(())
        } else {
            Err(Error::new(Errno::EACCES))
        }
    }
}

impl Drop for DrmFile {
    fn drop(&mut self) {
        // Release the magic through the file's retained context, which may no longer
        // be the device's current master.
        let mut auth_state = self.auth_state.lock();
        let master = auth_state.master.clone();
        if let (Some(master), Some(magic)) = (master, auth_state.magic.take()) {
            master.release_magic(magic);
        }
        drop(auth_state);

        let _ = self.device.drop_master(self.client_id);
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

    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        self.update_owner_process();

        dispatch_drm_ioctl!(
            self,
            match raw_ioctl {
                cmd @ DrmIoctlVersion => {
                    let mut args: DrmVersion = cmd.read()?;

                    let name = self.device.name();
                    let name_len = name.len();
                    let desc = self.device.desc();
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
                cmd @ DrmIoctlGetUnique => {
                    let mut args: DrmUnique = cmd.read()?;

                    // Linux keeps this empty until `DRM_IOCTL_SET_VERSION` has
                    // initialized the legacy bus ID for this master context.
                    // `SET_VERSION` is not implemented yet, so an empty value is
                    // the only compatible result.
                    args.unique_len = 0;
                    cmd.write(&args)?;
                    Ok(0)
                }
                cmd @ DrmIoctlGetMagic => {
                    let mut auth_state = self.auth_state.lock();
                    let magic = match auth_state.magic {
                        Some(magic) => magic,
                        None => {
                            let master = auth_state.master.as_ref().ok_or(Errno::EINVAL)?;
                            let magic = master.allocate_magic(&self.authenticated)?;
                            auth_state.magic = Some(magic);
                            magic
                        }
                    };
                    drop(auth_state);

                    let args = DrmAuth { magic };
                    cmd.write(&args)?;
                    Ok(0)
                }
                cmd @ DrmIoctlGetCap => {
                    use DrmGetCapability::*;

                    let mut args: DrmGetCap = cmd.read()?;
                    let cap = DrmGetCapability::try_from(args.capability)?;

                    let value = match cap {
                        TimestampMonotonic => 1,
                        Prime => (DrmPrimeValue::IMPORT | DrmPrimeValue::EXPORT).bits(),
                        SyncObj => self.has_feature(DrmFeatures::SYNCOBJ) as u64,
                        SyncObjTimeline => self.has_feature(DrmFeatures::SYNCOBJ_TIMELINE) as u64,
                        _ => {
                            if !self.has_feature(DrmFeatures::MODESET) {
                                return_errno!(Errno::EOPNOTSUPP);
                            }
                            let flags = self.device.caps().flags();
                            match cap {
                                DumbBuffer => flags.contains(DrmDeviceCapFlags::DUMB_BUFFER) as u64,
                                VblankHighCrtc => 1,
                                DumbPreferredDepth => {
                                    self.device.caps().preferred_color_depth() as u64
                                }
                                DumbPreferShadow => {
                                    flags.contains(DrmDeviceCapFlags::SHADOW_BUFFER) as u64
                                }
                                AsyncPageFlip => {
                                    flags.contains(DrmDeviceCapFlags::ASYNC_PAGE_FLIP) as u64
                                }
                                PageFlipTarget => {
                                    flags.contains(DrmDeviceCapFlags::PAGE_FLIP_TARGET) as u64
                                }
                                CursorWidth => self.device.caps().cursor_rect().width() as u64,
                                CursorHeight => self.device.caps().cursor_rect().height() as u64,
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
                                    let enabled = args.value >= 1;

                                    self.caps.has_atomic.store(enabled, Ordering::Relaxed);
                                    self.caps
                                        .has_universal_planes
                                        .store(enabled, Ordering::Relaxed);
                                    self.caps.has_aspect_ratio.store(enabled, Ordering::Relaxed);
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
                        CursorPlaneHotspot => {
                            if !self.has_feature(DrmFeatures::CURSOR_HOTSPOT) {
                                return_errno!(Errno::EOPNOTSUPP);
                            }

                            if !self.caps.has_atomic.load(Ordering::Relaxed) {
                                return_errno!(Errno::EINVAL);
                            }

                            match args.value {
                                0 | 1 => {
                                    self.caps
                                        .has_virtualized_cursor_plane
                                        .store(args.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            }
                        }
                    }
                    Ok(0)
                }
                cmd @ DrmIoctlAuthMagic => {
                    let args: DrmAuth = cmd.read()?;
                    let master = self.auth_state.lock().master.clone().ok_or(Errno::EINVAL)?;
                    master.authenticate_magic(args.magic)?;
                    Ok(0)
                }
                DrmIoctlSetMaster => {
                    self.check_master_control_permission()?;

                    let mut auth_state = self.auth_state.lock();
                    // A file becoming master for the first time replaces the context inherited
                    // at open with a context it owns. Since magic IDs are context-local, release
                    // any existing magic through the old context before replacing the reference.
                    let retained_master = auth_state
                        .was_master
                        .then_some(auth_state.master.as_ref())
                        .flatten();
                    let new_master = self.device.set_master(self.client_id, retained_master)?;

                    let old_master = auth_state.master.clone();
                    if let Some(old_master) = old_master
                        .as_ref()
                        .filter(|old_master| !Arc::ptr_eq(old_master, &new_master))
                    {
                        if let Some(magic) = auth_state.magic.take() {
                            old_master.release_magic(magic);
                        }
                    }

                    auth_state.was_master = true;
                    auth_state.master = Some(new_master);
                    self.authenticated.store(true, Ordering::Release);

                    Ok(0)
                }
                DrmIoctlDropMaster => {
                    self.check_master_control_permission()?;
                    self.device.drop_master(self.client_id)?;

                    Ok(0)
                }
                _ => {
                    ostd::debug!(
                        "the ioctl command {:#x} is unknown for DRM devices",
                        raw_ioctl.cmd()
                    );
                    return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown");
                }
            }
        )
    }
}
