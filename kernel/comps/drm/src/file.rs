// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use aster_core::{
    events::{IoEvents, PollHandle, Pollable},
    fs::{FileOps, Path, PerOpenFileOps, StatusFlags},
    ioctl::RawIoctl,
    prelude::*,
    process::Process,
};
use ostd::{
    mm::{VmReader, VmWriter},
    sync::Mutex,
};

use crate::{
    device::{DrmDevice, DrmFeatures, DrmMaster, RegisteredDrmDevice},
    has_current_sys_admin,
    minor::{DrmMinor, DrmMinorType},
};

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Default)]
pub struct DrmFileCaps {
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

impl DrmFileCaps {
    pub fn has_stereo(&self) -> bool {
        self.has_stereo.load(Ordering::Relaxed)
    }

    pub(crate) fn set_stereo(&self, enabled: bool) {
        self.has_stereo.store(enabled, Ordering::Relaxed);
    }

    pub fn has_universal_planes(&self) -> bool {
        self.has_universal_planes.load(Ordering::Relaxed)
    }

    pub(crate) fn set_universal_planes(&self, enabled: bool) {
        self.has_universal_planes.store(enabled, Ordering::Relaxed);
    }

    pub fn has_atomic(&self) -> bool {
        self.has_atomic.load(Ordering::Relaxed)
    }

    pub(crate) fn set_atomic(&self, enabled: bool) {
        self.has_atomic.store(enabled, Ordering::Relaxed);
        self.set_universal_planes(enabled);
        self.set_aspect_ratio(enabled);
    }

    pub fn has_aspect_ratio(&self) -> bool {
        self.has_aspect_ratio.load(Ordering::Relaxed)
    }

    pub(crate) fn set_aspect_ratio(&self, enabled: bool) {
        self.has_aspect_ratio.store(enabled, Ordering::Relaxed);
    }

    pub fn has_writeback_connectors(&self) -> bool {
        self.has_writeback_connectors.load(Ordering::Relaxed)
    }

    pub(crate) fn set_writeback_connectors(&self, enabled: bool) {
        self.has_writeback_connectors
            .store(enabled, Ordering::Relaxed);
    }

    pub fn has_virtualized_cursor_plane(&self) -> bool {
        self.has_virtualized_cursor_plane.load(Ordering::Relaxed)
    }

    pub(crate) fn set_virtualized_cursor_plane(&self, enabled: bool) {
        self.has_virtualized_cursor_plane
            .store(enabled, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
struct DrmFileAuthState {
    /// Tracks the current owner process for this file's master-management checks.
    ///
    /// For files that have never been master, this owner can follow the current
    /// ioctl caller (e.g., after fd passing). Once the file has been master,
    /// ownership is frozen to preserve "same process can reacquire master"
    /// semantics.
    owner_process: Option<Weak<Process>>,
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

/// Represents one open DRM file description.
///
/// It tracks per-open capabilities and authentication state and is passed to
/// driver ioctl handlers to provide the calling client's context.
#[derive(Debug)]
pub struct DrmFile {
    client_id: u64,
    caps: DrmFileCaps,
    auth_state: Mutex<DrmFileAuthState>,
    /// Authentication state referenced weakly by the master magic table.
    authenticated: Arc<AtomicBool>,

    minor: Arc<DrmMinor>,
    registered_device: Arc<RegisteredDrmDevice>,
}

impl DrmFile {
    pub fn device(&self) -> &Arc<dyn DrmDevice> {
        self.registered_device.device()
    }

    pub fn caps(&self) -> &DrmFileCaps {
        &self.caps
    }

    pub fn minor_type(&self) -> DrmMinorType {
        self.minor.type_()
    }

    pub fn has_feature(&self, feature: DrmFeatures) -> bool {
        self.registered_device.has_feature(feature)
    }

    pub(crate) fn new(minor: Arc<DrmMinor>) -> Self {
        let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
        let owner_process = Process::current().map(|process| Arc::downgrade(&process));
        let registered_device = minor.registered_device().clone();

        // Primary clients join the current master context. If no master exists,
        // the first primary client creates one and becomes master automatically.
        let (master, is_master) = match minor.type_() {
            DrmMinorType::Primary => {
                let (master, is_master) = registered_device.open_primary_client(client_id);
                (Some(master), is_master)
            }
            _ => (None, false),
        };

        let authenticated = Arc::new(AtomicBool::new(is_master));

        let auth_state = DrmFileAuthState {
            owner_process,
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
            registered_device,
        }
    }

    pub(crate) fn is_current_master(&self) -> bool {
        self.registered_device.is_current_master(self.client_id)
    }

    pub(crate) fn is_authenticated(&self) -> bool {
        self.is_current_master() || self.authenticated.load(Ordering::Acquire)
    }

    pub(crate) fn get_or_allocate_magic(&self) -> Result<u32> {
        let mut auth_state = self.auth_state.lock();
        if let Some(magic) = auth_state.magic {
            return Ok(magic);
        }

        let Some(master) = auth_state.master.as_ref() else {
            return_errno_with_message!(
                Errno::EINVAL,
                "the DRM file is not associated with a master"
            );
        };
        let magic = master.allocate_magic(&self.authenticated)?;
        auth_state.magic = Some(magic);
        Ok(magic)
    }

    pub(crate) fn authenticate_magic(&self, magic: u32) -> Result<()> {
        // Use device state to atomically verify the current master and authenticate the magic.
        self.registered_device
            .authenticate_magic(self.client_id, magic)
    }

    pub(crate) fn set_master(&self) -> Result<()> {
        self.check_master_control_permission()?;

        let mut auth_state = self.auth_state.lock();
        let retained_master = auth_state
            .was_master
            .then_some(auth_state.master.as_ref())
            .flatten();
        let new_master = self
            .registered_device
            .set_master(self.client_id, retained_master)?;

        let old_master = auth_state.master.clone();
        if let Some(old_master) = old_master
            .as_ref()
            .filter(|old_master| !Arc::ptr_eq(old_master, &new_master))
            && let Some(magic) = auth_state.magic.take()
        {
            old_master.release_magic(magic);
        }

        auth_state.was_master = true;
        auth_state.master = Some(new_master);
        self.authenticated.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn drop_master(&self) -> Result<()> {
        self.check_master_control_permission()?;
        self.registered_device.drop_master(self.client_id)
    }

    /// Keep tracking the ioctl caller while this file has never been master,
    /// so fd passing can update ownership. After the file has been master once,
    /// keep owner identity stable to enforce same-owner master reacquisition semantics.
    fn update_owner_process(&self) {
        let mut auth_state = self.auth_state.lock();

        if auth_state.was_master {
            return;
        }

        if let Some(process) = Process::current() {
            auth_state.owner_process = Some(Arc::downgrade(&process));
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
        let owner_process = auth_state.owner_process.as_ref().and_then(Weak::upgrade);
        let is_owner_process = owner_process
            .zip(Process::current())
            .is_some_and(|(owner, current)| Arc::ptr_eq(&owner, &current));

        if auth_state.was_master && is_owner_process {
            Ok(())
        } else {
            return_errno_with_message!(
                Errno::EACCES,
                "DRM master control requires CAP_SYS_ADMIN or ownership"
            )
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

        let _ = self.registered_device.drop_master(self.client_id);
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
