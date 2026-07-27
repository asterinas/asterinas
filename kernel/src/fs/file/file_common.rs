// SPDX-License-Identifier: MPL-2.0

//! State shared by all references to a file description.

use core::sync::atomic::Ordering;

use super::{AtomicStatusFlags, FileLike, StatusFlags, file_handle::StatusFlagsUpdate};
use crate::{
    events::{IoEvents, Observer},
    fs::vfs::path::Path,
    prelude::*,
    process::{
        Pid, Process,
        signal::{PollAdaptor, constants::SIGIO, sig_num::SigNum},
    },
};

/// Common fields for a file description.
///
/// This type is intended to collect state that belongs to the file description rather than to a
/// specific file descriptor.
pub struct FileCommon {
    path: Path,
    status_flags: AtomicStatusFlags,
    owner: FileOwner,
}

impl FileCommon {
    /// Creates common state for a file description.
    pub fn new(path: Path, status_flags: StatusFlags) -> Self {
        Self {
            path,
            status_flags: AtomicStatusFlags::new(status_flags),
            owner: FileOwner::new(),
        }
    }

    /// Returns the path associated with the file description.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the current file status flags.
    pub fn status_flags(&self) -> StatusFlags {
        self.status_flags.load(Ordering::Relaxed)
    }

    /// Returns whether the file description is in non-blocking mode.
    pub fn is_nonblocking(&self) -> bool {
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
                owner.register_observer(file, self.owner.sigio_signum.clone());
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
    pub fn owner(&self) -> &FileOwner {
        &self.owner
    }
}

/// The process that receives asynchronous I/O signals for a file description.
pub struct FileOwner {
    inner: Mutex<Option<Owner>>,
    /// The signal number for async I/O notifications.
    /// `None` means no custom signal has been set; the default signal is used.
    sigio_signum: Arc<Mutex<Option<SigNum>>>,
}

impl FileOwner {
    /// Creates an owner state with no process assigned.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            sigio_signum: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the process ID of the current owner.
    pub fn pid(&self) -> Option<Pid> {
        self.inner.lock().as_ref().map(|owner| owner.pid)
    }

    /// Returns the custom async I/O signal number, or `None` if not set.
    pub fn sigio_signum(&self) -> Option<SigNum> {
        *self.sigio_signum.lock()
    }

    /// Sets the custom async I/O signal number.
    /// Passing `None` resets to the default behavior.
    pub fn set_sigio_signum(&self, signal: Option<SigNum>) {
        *self.sigio_signum.lock() = signal;
    }

    pub(super) fn set(&self, file: &dyn FileLike, owner: Option<&Arc<Process>>) {
        let mut owner_guard = self.inner.lock();
        *owner_guard = None;

        let Some(process) = owner else {
            return;
        };

        let mut owner = Owner::new(process);
        if file.status_flags().contains(StatusFlags::O_ASYNC) {
            owner.register_observer(file, self.sigio_signum.clone());
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
    pid: Pid,
    process: Weak<Process>,
    poller: Option<PollAdaptor<OwnerObserver>>,
}

impl Owner {
    fn new(process: &Arc<Process>) -> Self {
        Self {
            pid: process.pid(),
            process: Arc::downgrade(process),
            poller: None,
        }
    }

    fn register_observer(&mut self, file: &dyn FileLike, sigio_signum: Arc<Mutex<Option<SigNum>>>) {
        if self.poller.is_some() {
            return;
        }

        let mut poller =
            PollAdaptor::with_observer(OwnerObserver::new(self.process.clone(), sigio_signum));
        file.poll(IoEvents::IN | IoEvents::OUT, Some(poller.as_handle_mut()));
        self.poller = Some(poller);
    }

    fn unregister_observer(&mut self) {
        self.poller = None;
    }
}

struct OwnerObserver {
    owner: Weak<Process>,
    sigio_signum: Arc<Mutex<Option<SigNum>>>,
}

impl OwnerObserver {
    fn new(owner: Weak<Process>, sigio_signum: Arc<Mutex<Option<SigNum>>>) -> Self {
        Self {
            owner,
            sigio_signum,
        }
    }
}

impl Observer<IoEvents> for OwnerObserver {
    fn on_events(&self, _events: &IoEvents) {
        let signum = self.sigio_signum.lock().unwrap_or(SIGIO);
        crate::process::enqueue_signal_async(self.owner.clone(), signum);
    }
}
