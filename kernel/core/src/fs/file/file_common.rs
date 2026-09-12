// SPDX-License-Identifier: MPL-2.0

//! State shared by all references to a file description.

use core::sync::atomic::Ordering;

use super::{AccessMode, AtomicStatusFlags, FileLike, StatusFlags, file_handle::StatusFlagsUpdate};
use crate::{
    events::{IoEvents, Observer},
    fs::vfs::{notify, path::Path},
    prelude::*,
    process::{
        FileOwnerCreds, Process, ProcessGroup, broadcast_sigio_async, enqueue_sigio_async,
        enqueue_sigio_to_thread_async, posix_thread::AsPosixThread, signal::PollAdaptor,
    },
    thread::Thread,
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
        if let Some(owner) = owner_guard.owner.as_mut() {
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

/// The thread, process or process group that receives asynchronous I/O signals for a file
/// description.
pub(crate) struct FileOwner {
    inner: Mutex<FileOwnerInner>,
}

#[derive(Default)]
struct FileOwnerInner {
    /// The kind requested by the most recent `F_SETOWN`/`F_SETOWN_EX`.
    ///
    /// This is kept even when the owner is cleared, because Linux keeps reporting the
    /// recorded type from `F_GETOWN_EX` and only zeroes the identifier. `None` means the
    /// owner was never set at all, which Linux reports as `F_OWNER_TID`.
    kind: Option<FileOwnerKind>,
    owner: Option<Owner>,
}

/// The kind of entity that owns a file description, as reported by `F_GETOWN_EX`.
///
/// The discriminants are the `F_OWNER_*` constants, so this converts straight to and from
/// the `type` field of `struct f_owner_ex`.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum FileOwnerKind {
    /// A single thread. `F_OWNER_TID`.
    Thread = 0,
    /// A single process. `F_OWNER_PID`.
    Process = 1,
    /// A process group. `F_OWNER_PGRP`.
    ProcessGroup = 2,
}

/// The recipient of the asynchronous I/O signals for a file description.
///
/// A file description is owned by a single thread, a single process, or an entire process
/// group. In the last case every member of the group receives the signal.
pub(crate) enum FileOwnerTarget {
    /// A single thread, selectable only through `F_SETOWN_EX` with `F_OWNER_TID`.
    Thread(Arc<Thread>),
    /// A single process, selected by a positive `fcntl(F_SETOWN)` argument or by
    /// `F_SETOWN_EX` with `F_OWNER_PID`.
    Process(Arc<Process>),
    /// A process group, selected by a negative `fcntl(F_SETOWN)` argument or by
    /// `F_SETOWN_EX` with `F_OWNER_PGRP`.
    ProcessGroup(Arc<ProcessGroup>),
}

impl FileOwnerTarget {
    fn downgrade(&self) -> WeakFileOwnerTarget {
        match self {
            Self::Thread(thread) => WeakFileOwnerTarget::Thread(Arc::downgrade(thread)),
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
    Thread(Weak<Thread>),
    Process(Weak<Process>),
    ProcessGroup(Weak<ProcessGroup>),
}

impl WeakFileOwnerTarget {
    /// Returns the identifier of the owner, or `None` if the owner no longer exists.
    ///
    /// Linux reports zero once the owning task is gone rather than a stale identifier,
    /// which is what the `None` here becomes.
    fn id(&self) -> Option<i32> {
        match self {
            // A TID and a PID never exceed `i32::MAX`, so the casts preserve the value.
            Self::Thread(thread) => Some(thread.upgrade()?.as_posix_thread()?.tid() as i32),
            Self::Process(process) => Some(process.upgrade()?.pid() as i32),
            // A PGID is the PID of the group leader, and is reported negated.
            Self::ProcessGroup(group) => Some(-(group.upgrade()?.pgid() as i32)),
        }
    }
}

impl FileOwner {
    /// Creates an owner state with no process or process group assigned.
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(FileOwnerInner::default()),
        }
    }

    /// Returns the identifier of the current owner.
    ///
    /// A thread or process ID is returned as a positive value and a process group ID as a
    /// negative value. `None` means that the file description has no owner, or that the
    /// owner it had has since exited.
    pub(crate) fn id(&self) -> Option<i32> {
        self.inner
            .lock()
            .owner
            .as_ref()
            .and_then(|owner| owner.target.id())
    }

    /// Returns the kind of the current owner, or `None` if there is no owner.
    ///
    /// Unlike [`Self::id`], this survives the owner exiting: `F_GETOWN_EX` keeps reporting
    /// the recorded type and only zeroes the identifier.
    pub(crate) fn kind(&self) -> Option<FileOwnerKind> {
        self.inner.lock().kind
    }

    /// `kind` is recorded even when `owner` is `None`, so that clearing the owner leaves
    /// the reported type behind the way Linux does.
    pub(super) fn set(
        &self,
        file: &dyn FileLike,
        owner: Option<&FileOwnerTarget>,
        kind: FileOwnerKind,
        creds: FileOwnerCreds,
    ) {
        let mut inner = self.inner.lock();
        inner.owner = None;
        inner.kind = Some(kind);

        let Some(target) = owner else {
            return;
        };

        let mut new_owner = Owner::new(target, creds);
        if file.status_flags().contains(StatusFlags::O_ASYNC) {
            new_owner.register_observer(file);
        }
        inner.owner = Some(new_owner);
    }
}

impl Default for FileOwner {
    fn default() -> Self {
        Self::new()
    }
}

struct Owner {
    target: WeakFileOwnerTarget,
    /// The credentials of whoever called `fcntl(F_SETOWN)`, recorded at that moment.
    creds: FileOwnerCreds,
    poller: Option<PollAdaptor<OwnerObserver>>,
}

impl Owner {
    fn new(target: &FileOwnerTarget, creds: FileOwnerCreds) -> Self {
        Self {
            target: target.downgrade(),
            creds,
            poller: None,
        }
    }

    fn register_observer(&mut self, file: &dyn FileLike) {
        if self.poller.is_some() {
            return;
        }

        let mut poller =
            PollAdaptor::with_observer(OwnerObserver::new(self.target.clone(), self.creds));
        file.poll(IoEvents::IN | IoEvents::OUT, Some(poller.as_handle_mut()));
        self.poller = Some(poller);
    }

    fn unregister_observer(&mut self) {
        self.poller = None;
    }
}

struct OwnerObserver {
    owner: WeakFileOwnerTarget,
    creds: FileOwnerCreds,
}

impl OwnerObserver {
    fn new(owner: WeakFileOwnerTarget, creds: FileOwnerCreds) -> Self {
        Self { owner, creds }
    }
}

impl Observer<IoEvents> for OwnerObserver {
    fn on_events(&self, _events: &IoEvents) {
        // Delivery is subject to the same permission check as `kill`, evaluated against the
        // credentials saved at `fcntl(F_SETOWN)` time rather than against whoever happens to
        // be running now. The check itself happens in the work item, since reading another
        // process's credentials is not possible here in atomic mode.
        match &self.owner {
            WeakFileOwnerTarget::Thread(thread) => {
                enqueue_sigio_to_thread_async(thread.clone(), self.creds);
            }
            WeakFileOwnerTarget::Process(process) => {
                enqueue_sigio_async(process.clone(), self.creds);
            }
            WeakFileOwnerTarget::ProcessGroup(group) => {
                broadcast_sigio_async(group.clone(), self.creds);
            }
        }
    }
}
