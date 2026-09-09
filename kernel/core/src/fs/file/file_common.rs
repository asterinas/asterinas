// SPDX-License-Identifier: MPL-2.0

//! State shared by all references to a file description.

use core::sync::atomic::Ordering;

use super::{AccessMode, AtomicStatusFlags, FileLike, StatusFlags, file_handle::StatusFlagsUpdate};
use crate::{
    events::{IoEvents, Observer},
    fs::vfs::{notify, path::Path},
    prelude::*,
    process::{
        Process, ProcessGroup, broadcast_signal_async, enqueue_signal_async,
        signal::{PollAdaptor, constants::SIGIO},
    },
};

/// Common fields for a file description.
///
/// This type is intended to collect state that belongs to the file description rather than to a
/// specific file descriptor.
pub(crate) struct FileCommon {
    path: Path,
    access_mode: AccessMode,
    status_flags: AtomicStatusFlags,
    owner: FileOwner,
}

impl FileCommon {
    /// Creates common state for a file description.
    pub(crate) fn new(path: Path, access_mode: AccessMode, status_flags: StatusFlags) -> Self {
        Self {
            path,
            access_mode,
            status_flags: AtomicStatusFlags::new(status_flags),
            owner: FileOwner::new(),
        }
    }

    /// Returns the path associated with the file description.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the access mode of the file description.
    pub(crate) fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    /// Returns the current file status flags.
    pub(crate) fn status_flags(&self) -> StatusFlags {
        self.status_flags.load(Ordering::Relaxed)
    }

    /// Returns whether the file description is in non-blocking mode.
    pub(crate) fn is_nonblocking(&self) -> bool {
        self.status_flags().contains(StatusFlags::O_NONBLOCK)
    }

    /// Atomically updates the file status flags.
    pub(super) fn update_status_flags(&self, file: &dyn FileLike, update: StatusFlagsUpdate) {
        if !update.affects(StatusFlags::O_ASYNC) {
            self.apply_status_flags(update);
            return;
        }

        let mut owner_guard = self.owner.inner.lock();
        if let Some(owner) = owner_guard.as_mut() {
            if update.flags().contains(StatusFlags::O_ASYNC) {
                owner.register_observer(file);
            } else {
                owner.unregister_observer();
            }
        }
        self.apply_status_flags(update);
    }

    fn apply_status_flags(&self, update: StatusFlagsUpdate) {
        self.status_flags
            .update(Ordering::Relaxed, Ordering::Relaxed, |status_flags| {
                update.apply(status_flags)
            });
    }

    /// Returns the asynchronous I/O signal owner.
    pub(crate) fn owner(&self) -> &FileOwner {
        &self.owner
    }
}

impl Drop for FileCommon {
    fn drop(&mut self) {
        notify::on_close(self);
    }
}

/// The process or process group that receives asynchronous I/O signals for a file description.
pub(crate) struct FileOwner {
    inner: Mutex<Option<Owner>>,
}

/// The recipient of the asynchronous I/O signals for a file description.
///
/// A file description is owned either by a single process or by an entire process group. In the
/// latter case every member of the group receives the signal.
pub(crate) enum FileOwnerTarget {
    /// A single process, selected by a positive `fcntl(F_SETOWN)` argument.
    Process(Arc<Process>),
    /// A process group, selected by a negative `fcntl(F_SETOWN)` argument.
    ProcessGroup(Arc<ProcessGroup>),
}

impl FileOwnerTarget {
    /// Returns the identifier that `fcntl(F_GETOWN)` reports for this owner.
    ///
    /// A process ID is reported as a positive value and a process group ID as a negative value.
    fn id(&self) -> i32 {
        match self {
            // A PID never exceeds `i32::MAX`, so the cast preserves the value.
            Self::Process(process) => process.pid() as i32,
            // Likewise for a PGID, which is the PID of the group leader.
            Self::ProcessGroup(group) => -(group.pgid() as i32),
        }
    }

    fn downgrade(&self) -> WeakFileOwnerTarget {
        match self {
            Self::Process(process) => WeakFileOwnerTarget::Process(Arc::downgrade(process)),
            Self::ProcessGroup(group) => WeakFileOwnerTarget::ProcessGroup(Arc::downgrade(group)),
        }
    }
}

/// A weak reference to a [`FileOwnerTarget`].
///
/// The owner must not be kept alive by the file description that it owns.
#[derive(Clone)]
enum WeakFileOwnerTarget {
    Process(Weak<Process>),
    ProcessGroup(Weak<ProcessGroup>),
}

impl FileOwner {
    /// Creates an owner state with no process or process group assigned.
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Returns the identifier of the current owner.
    ///
    /// A process ID is returned as a positive value and a process group ID as a negative value.
    /// `None` means that the file description has no owner.
    pub(crate) fn id(&self) -> Option<i32> {
        self.inner.lock().as_ref().map(|owner| owner.id)
    }

    pub(super) fn set(&self, file: &dyn FileLike, owner: Option<&FileOwnerTarget>) {
        let mut owner_guard = self.inner.lock();
        *owner_guard = None;

        let Some(target) = owner else {
            return;
        };

        let mut owner = Owner::new(target);
        if file.status_flags().contains(StatusFlags::O_ASYNC) {
            owner.register_observer(file);
        }
        *owner_guard = Some(owner);
    }
}

impl Default for FileOwner {
    fn default() -> Self {
        Self::new()
    }
}

struct Owner {
    id: i32,
    target: WeakFileOwnerTarget,
    poller: Option<PollAdaptor<OwnerObserver>>,
}

impl Owner {
    fn new(target: &FileOwnerTarget) -> Self {
        Self {
            id: target.id(),
            target: target.downgrade(),
            poller: None,
        }
    }

    fn register_observer(&mut self, file: &dyn FileLike) {
        if self.poller.is_some() {
            return;
        }

        let mut poller = PollAdaptor::with_observer(OwnerObserver::new(self.target.clone()));
        file.poll(IoEvents::IN | IoEvents::OUT, Some(poller.as_handle_mut()));
        self.poller = Some(poller);
    }

    fn unregister_observer(&mut self) {
        self.poller = None;
    }
}

struct OwnerObserver {
    owner: WeakFileOwnerTarget,
}

impl OwnerObserver {
    fn new(owner: WeakFileOwnerTarget) -> Self {
        Self { owner }
    }
}

impl Observer<IoEvents> for OwnerObserver {
    fn on_events(&self, _events: &IoEvents) {
        match &self.owner {
            WeakFileOwnerTarget::Process(process) => {
                enqueue_signal_async(process.clone(), SIGIO);
            }
            WeakFileOwnerTarget::ProcessGroup(group) => {
                broadcast_signal_async(group.clone(), SIGIO);
            }
        }
    }
}
