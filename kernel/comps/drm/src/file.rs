// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use aster_core::{
    current,
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
use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::Mutex,
};

use crate::{
    device::{DrmDevice, DrmFeatures, DrmMaster},
    has_current_sys_admin,
    minor::{DrmMinor, DrmMinorType},
};

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

bitflags::bitflags! {
    pub struct DrmFileCaps: u32 {
        /// The client has asked us to expose stereo 3D mode flags.
        const STEREO_3D = 1 << 0;
        /// The client understands CRTC primary and cursor planes in the plane list.
        const UNIVERSAL_PLANES = 1 << 1;
        /// The client understands atomic properties.
        const ATOMIC = 1 << 2;
        /// The client can handle picture aspect ratios.
        const ASPECT_RATIO = 1 << 3;
        /// The client understands writeback connectors.
        const WRITEBACK_CONNECTORS = 1 << 4;
        /// The client can handle cursor-plane hotspots for virtualized drivers.
        const CURSOR_PLANE_HOTSPOT = 1 << 5;
    }
}

impl From<u32> for DrmFileCaps {
    fn from(value: u32) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<DrmFileCaps> for u32 {
    fn from(value: DrmFileCaps) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(DrmFileCaps, {
    /// An atomic version of [`DrmFileCaps`].
    #[derive(Debug, Default)]
    struct AtomicDrmFileCaps(AtomicU32);
});

impl AtomicDrmFileCaps {
    fn set(&self, flags: DrmFileCaps, enabled: bool) {
        self.update(Ordering::Relaxed, Ordering::Relaxed, |mut caps| {
            caps.set(flags, enabled);
            caps
        });
    }
}

#[derive(Debug)]
struct DrmPrimaryAuthState {
    /// Tracks the current owner process for this file's master-management checks.
    ///
    /// For files that have never been master, this owner can follow the current
    /// ioctl caller (e.g., after fd passing). Once the file has been master,
    /// ownership is frozen to preserve "same process can reacquire master"
    /// semantics.
    owner_process: Weak<Process>,
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
    master: Arc<DrmMaster>,
}

/// Authentication and master state associated with a primary DRM file.
#[derive(Debug)]
struct DrmPrimaryAuth {
    state: Mutex<DrmPrimaryAuthState>,
    /// Authentication state referenced weakly by the master magic table.
    authenticated: Arc<AtomicBool>,
}

/// Represents one open DRM file description.
///
/// It tracks per-open capabilities and authentication state and is passed to
/// driver ioctl handlers to provide the calling client's context.
#[derive(Debug)]
pub struct DrmFile {
    client_id: u64,
    file_caps: AtomicDrmFileCaps,
    /// Authentication state present only for primary-node files.
    auth: Option<DrmPrimaryAuth>,
    minor: Arc<DrmMinor>,
}

impl DrmFile {
    pub fn device(&self) -> &Arc<dyn DrmDevice> {
        self.minor.device()
    }

    pub fn file_caps(&self) -> DrmFileCaps {
        self.file_caps.load(Ordering::Relaxed)
    }

    pub fn minor_type(&self) -> DrmMinorType {
        self.minor.type_()
    }

    pub fn has_features(&self, feature: DrmFeatures) -> bool {
        self.device().has_features(feature)
    }

    pub(super) fn new(minor: Arc<DrmMinor>) -> Self {
        let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
        let auth = minor.open_client(client_id).map(|master| {
            let is_master = master.owner_client_id() == client_id;

            DrmPrimaryAuth {
                state: Mutex::new(DrmPrimaryAuthState {
                    owner_process: Arc::downgrade(&current!()),
                    was_master: is_master,
                    magic: None,
                    master,
                }),
                authenticated: Arc::new(AtomicBool::new(is_master)),
            }
        });

        Self {
            client_id,
            file_caps: AtomicDrmFileCaps::default(),
            auth,
            minor,
        }
    }

    pub(super) fn is_master(&self) -> bool {
        self.minor.is_client_master(self.client_id)
    }

    pub(super) fn set_caps(&self, caps: DrmFileCaps, enabled: bool) {
        self.file_caps.set(caps, enabled);
    }

    pub(super) fn is_authenticated(&self) -> bool {
        self.auth
            .as_ref()
            .is_some_and(|auth| auth.authenticated.load(Ordering::Relaxed))
    }

    pub(super) fn get_or_allocate_magic(&self) -> Result<u32> {
        let auth = self.primary_auth()?;
        let mut auth_state = auth.state.lock();
        if let Some(magic) = auth_state.magic {
            return Ok(magic);
        }

        let magic = auth_state.master.allocate_magic(&auth.authenticated)?;
        auth_state.magic = Some(magic);
        Ok(magic)
    }

    pub(super) fn authenticate_magic(&self, magic: u32) -> Result<()> {
        self.minor.authenticate_magic(self.client_id, magic)
    }

    pub(super) fn set_master(&self) -> Result<()> {
        self.check_master_control_permission()?;

        let auth = self.primary_auth()?;
        let mut auth_state = auth.state.lock();
        let retained_master = auth_state.was_master.then_some(&auth_state.master);
        let master = self.minor.set_master(self.client_id, retained_master)?;

        if !Arc::ptr_eq(&auth_state.master, &master)
            && let Some(magic) = auth_state.magic.take()
        {
            auth_state.master.release_magic(magic);
        }

        auth_state.was_master = true;
        auth_state.master = master;
        auth.authenticated.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub(super) fn drop_master(&self) -> Result<()> {
        self.check_master_control_permission()?;
        self.minor.drop_master(self.client_id)
    }

    /// Keeps tracking the ioctl caller while this file has never been master,
    /// so fd passing can update ownership. After the file has been master once,
    /// keeps owner identity stable to enforce same-owner master reacquisition semantics.
    fn update_owner_process(&self) {
        let Some(auth) = self.auth.as_ref() else {
            return;
        };
        let mut auth_state = auth.state.lock();

        if auth_state.was_master {
            return;
        }

        auth_state.owner_process = Arc::downgrade(&current!());
    }

    /// Checks permission to change DRM master ownership.
    ///
    /// `CAP_SYS_ADMIN` may always perform the operation.
    /// Otherwise, only the same process acting through a file that
    /// is or was master is permitted, preserving master control across
    /// logind-style file descriptor passing.
    fn check_master_control_permission(&self) -> Result<()> {
        if has_current_sys_admin() {
            return Ok(());
        }

        let auth = self.primary_auth()?;
        let auth_state = auth.state.lock();
        let is_owner_process = auth_state
            .owner_process
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, &current!()));

        if auth_state.was_master && is_owner_process {
            Ok(())
        } else {
            return_errno_with_message!(
                Errno::EACCES,
                "the DRM master control requires CAP_SYS_ADMIN or ownership"
            )
        }
    }

    fn primary_auth(&self) -> Result<&DrmPrimaryAuth> {
        self.auth.as_ref().ok_or_else(|| {
            Error::with_message(
                Errno::EACCES,
                "the DRM operation requires a primary-node file",
            )
        })
    }
}

impl Drop for DrmFile {
    fn drop(&mut self) {
        let Some(auth) = self.auth.as_ref() else {
            return;
        };

        // Releases the magic through the file's retained context, which may no longer
        // be the device's current master.
        let mut auth_state = auth.state.lock();
        if let Some(magic) = auth_state.magic.take() {
            auth_state.master.release_magic(magic);
        }
        drop(auth_state);

        let _ = self.minor.drop_master(self.client_id);
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
        return_errno_with_message!(Errno::EINVAL, "reading from a DRM file is not supported")
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "writing to a DRM file is not supported")
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
        self.dispatch_ioctl(raw_ioctl)
    }
}
