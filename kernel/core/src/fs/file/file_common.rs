// SPDX-License-Identifier: MPL-2.0

//! State shared by all references to a file description.

use core::sync::atomic::Ordering;

use aster_rights::ReadOp;
use ostd::task::Task;

use super::{AccessMode, AtomicStatusFlags, FileLike, StatusFlags, file_handle::StatusFlagsUpdate};
use crate::{
    events::{IoEvents, Observer},
    fs::vfs::{notify, path::Path},
    prelude::*,
    process::{
        Credentials, Pid, Process,
        posix_thread::AsPosixThread,
        signal::{PollAdaptor, constants::SIGIO},
    },
};

/// Common fields for a file description.
///
/// This type is intended to collect state that belongs to the file description rather than to a
/// specific file descriptor.
pub(crate) struct FileCommon {
    open_cred: Option<Credentials<ReadOp>>,
    path: Path,
    access_mode: AccessMode,
    status_flags: AtomicStatusFlags,
    owner: FileOwner,
}

impl FileCommon {
    /// Creates common state for a file description.
    pub(crate) fn new(path: Path, access_mode: AccessMode, status_flags: StatusFlags) -> Self {
        let open_cred = Task::current().and_then(|task| {
            let posix_thread = task.as_posix_thread()?;
            Some(posix_thread.credentials())
        });

        Self::new_with_cred(path, access_mode, status_flags, open_cred)
    }

    fn new_with_cred(
        path: Path,
        access_mode: AccessMode,
        status_flags: StatusFlags,
        open_cred: Option<Credentials<ReadOp>>,
    ) -> Self {
        // Duplicating the thread's credentials would share mutable state with it.
        // Keep an independent snapshot so later credential changes cannot affect this file.
        let open_cred = open_cred.as_ref().map(Credentials::new_from);

        Self {
            open_cred,
            path,
            access_mode,
            status_flags: AtomicStatusFlags::new(status_flags),
            owner: FileOwner::new(),
        }
    }

    /// Returns the opener's credentials captured when this file description was created.
    ///
    /// Returns `None` if the file description was created without a current POSIX thread.
    #[cfg_attr(
        not(ktest),
        expect(
            dead_code,
            reason = "will be used by permission checks based on file-opening credentials"
        )
    )]
    pub(crate) fn open_cred(&self) -> Option<&Credentials<ReadOp>> {
        self.open_cred.as_ref()
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

/// The process that receives asynchronous I/O signals for a file description.
pub(crate) struct FileOwner {
    inner: Mutex<Option<Owner>>,
}

impl FileOwner {
    /// Creates an owner state with no process assigned.
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Returns the process ID of the current owner.
    pub(crate) fn pid(&self) -> Option<Pid> {
        self.inner.lock().as_ref().map(|owner| owner.pid)
    }

    pub(super) fn set(&self, file: &dyn FileLike, owner: Option<&Arc<Process>>) {
        let mut owner_guard = self.inner.lock();
        *owner_guard = None;

        let Some(process) = owner else {
            return;
        };

        let mut owner = Owner::new(process);
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

    fn register_observer(&mut self, file: &dyn FileLike) {
        if self.poller.is_some() {
            return;
        }

        let mut poller = PollAdaptor::with_observer(OwnerObserver::new(self.process.clone()));
        file.poll(IoEvents::IN | IoEvents::OUT, Some(poller.as_handle_mut()));
        self.poller = Some(poller);
    }

    fn unregister_observer(&mut self) {
        self.poller = None;
    }
}

struct OwnerObserver {
    owner: Weak<Process>,
}

impl OwnerObserver {
    fn new(owner: Weak<Process>) -> Self {
        Self { owner }
    }
}

impl Observer<IoEvents> for OwnerObserver {
    fn on_events(&self, _events: &IoEvents) {
        crate::process::enqueue_signal_async(self.owner.clone(), SIGIO);
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;
    use crate::{
        fs::pseudofs::SockFs,
        process::{Gid, Uid, credentials::capabilities::CapSet},
    };

    fn new_path() -> Path {
        crate::time::clocks::init_for_ktest();
        SockFs::new_path()
    }

    fn new_credentials() -> Credentials {
        Credentials::new_for_ktest(
            Uid::new(1000),
            Gid::new(2000),
            CapSet::CHOWN | CapSet::SETUID | CapSet::SETGID,
        )
    }

    fn assert_ids(cred: &Credentials<ReadOp>, uid: Uid, gid: Gid) {
        assert_eq!(cred.ruid(), uid);
        assert_eq!(cred.euid(), uid);
        assert_eq!(cred.suid(), uid);
        assert_eq!(cred.fsuid(), uid);
        assert_eq!(cred.rgid(), gid);
        assert_eq!(cred.egid(), gid);
        assert_eq!(cred.sgid(), gid);
        assert_eq!(cred.fsgid(), gid);
    }

    #[ktest]
    fn opening_credentials_survive_credential_changes() {
        let credentials = new_credentials();
        credentials.groups_mut().insert(Gid::new(2002));
        let original_caps = credentials.effective_capset().bits();
        let file = FileCommon::new_with_cred(
            new_path(),
            AccessMode::O_RDONLY,
            StatusFlags::empty(),
            Some(credentials.dup().restrict()),
        );

        credentials.set_uid(Uid::new(1001)).unwrap();
        credentials.set_gid(Gid::new(2001)).unwrap();
        *credentials.groups_mut() = BTreeSet::from([Gid::new(2003)]);
        credentials.set_effective_capset(CapSet::empty());
        credentials.set_permitted_capset(CapSet::empty());

        assert_ids(
            &credentials.dup().restrict(),
            Uid::new(1001),
            Gid::new(2001),
        );
        assert_eq!(*credentials.groups(), BTreeSet::from([Gid::new(2003)]));
        assert!(credentials.effective_capset().is_empty());
        assert!(credentials.permitted_capset().is_empty());
        drop(credentials);

        let saved = file.open_cred().unwrap();
        assert_ids(saved, Uid::new(1000), Gid::new(2000));
        assert_eq!(
            *saved.groups(),
            BTreeSet::from([Gid::new(2000), Gid::new(2002)])
        );
        assert_eq!(saved.effective_capset().bits(), original_caps);
        assert_eq!(saved.permitted_capset().bits(), original_caps);
    }

    #[ktest]
    fn separate_opens_capture_separate_credentials() {
        let credentials = new_credentials();
        let path = new_path();
        let first = Arc::new(FileCommon::new_with_cred(
            path.clone(),
            AccessMode::O_RDONLY,
            StatusFlags::empty(),
            Some(credentials.dup().restrict()),
        ));
        let shared = first.clone();

        credentials.set_uid(Uid::new(1001)).unwrap();
        credentials.set_gid(Gid::new(2001)).unwrap();
        let second = FileCommon::new_with_cred(
            path,
            AccessMode::O_RDONLY,
            StatusFlags::empty(),
            Some(credentials.dup().restrict()),
        );
        drop(first);
        drop(credentials);

        assert_ids(shared.open_cred().unwrap(), Uid::new(1000), Gid::new(2000));
        assert_ids(second.open_cred().unwrap(), Uid::new(1001), Gid::new(2001));
    }

    #[ktest]
    fn files_without_posix_openers_have_no_credentials() {
        assert!(Task::current().unwrap().as_posix_thread().is_none());
        let file = FileCommon::new(new_path(), AccessMode::O_RDONLY, StatusFlags::empty());

        assert!(file.open_cred().is_none());
    }
}
