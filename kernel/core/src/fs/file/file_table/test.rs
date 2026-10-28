// SPDX-License-Identifier: MPL-2.0

use ostd::prelude::*;

use super::*;
use crate::{
    events::IoEvents,
    fs::file::FileCommon,
    process::signal::{PollHandle, Pollable},
};

struct TestFile;

impl Pollable for TestFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileLike for TestFile {
    fn common(&self) -> &FileCommon {
        unreachable!("descriptor tests do not perform file I/O")
    }

    fn dump_proc_fdinfo(self: Arc<Self>, _fd_flags: FdFlags) -> Box<dyn Display> {
        Box::new("")
    }
}

fn file() -> Arc<dyn FileLike> {
    Arc::new(TestFile)
}

fn fd(raw: RawFileDesc) -> FileDesc {
    raw.try_into().unwrap()
}

#[ktest]
fn reserved_fd_is_invisible_until_installation() {
    let table = FileTable::new();
    let reservation = FileDescReservation::new(&table, 4).unwrap();
    {
        let mut locked = table.write();
        assert_eq!(locked.get_file(fd(0)).err().unwrap().error(), Errno::EBADF);
        assert_eq!(locked.get_entry(fd(0)).err().unwrap().error(), Errno::EBADF);
        assert!(locked.close_file(fd(0)).is_none());
        assert_eq!(locked.fds_and_files().count(), 0);
        assert_eq!(
            locked
                .insert_ceil_with_limit(file(), None, FdFlags::empty(), 4)
                .ok(),
            Some(fd(1))
        );
    }
    let installed = file();
    let installed_fd = reservation.install(installed.clone(), FdFlags::CLOEXEC);
    assert_eq!(installed_fd, fd(0));
    let locked = table.read();
    assert!(Arc::ptr_eq(
        locked.get_file(installed_fd).unwrap(),
        &installed
    ));
    assert!(
        locked
            .get_entry(installed_fd)
            .unwrap()
            .flags()
            .contains(FdFlags::CLOEXEC)
    );
}

#[ktest]
fn abandoned_fd_reservation_releases_its_slot() {
    let table = FileTable::new();
    let first = FileDescReservation::new(&table, 2).unwrap();
    let second = FileDescReservation::new(&table, 2).unwrap();
    assert_eq!(
        FileDescReservation::new(&table, 2).err().unwrap().error(),
        Errno::EMFILE
    );
    drop(first);
    let replacement = FileDescReservation::new(&table, 1).unwrap();
    assert_eq!(replacement.install(file(), FdFlags::empty()), fd(0));
    drop(second);
    assert_eq!(
        table
            .write()
            .insert_ceil_with_limit(file(), None, FdFlags::empty(), 2)
            .ok(),
        Some(fd(1))
    );
}

#[ktest]
fn dup_skips_reservations_and_rejects_overwriting_them() {
    let table = FileTable::new();
    let source = table
        .write()
        .insert_ceil_with_limit(file(), None, FdFlags::empty(), 4)
        .ok()
        .unwrap();
    let reserved = FileDescReservation::new(&table, 4).unwrap();
    {
        let mut locked = table.write();
        assert_eq!(
            locked
                .dup_exact_with_limit(source, fd(1), FdFlags::empty(), 4)
                .err()
                .unwrap()
                .error(),
            Errno::EBUSY
        );
        let duplicate = locked.get_file(source).unwrap().clone();
        assert_eq!(
            locked
                .insert_ceil_with_limit(duplicate, Some(fd(0)), FdFlags::empty(), 4)
                .ok(),
            Some(fd(2))
        );
        assert_eq!(locked.get_file(fd(1)).err().unwrap().error(), Errno::EBADF);
    }
    assert_eq!(u32::from(reserved.install(file(), FdFlags::CLOEXEC)), 1);
}

#[ktest]
fn fork_omits_reservations_but_preserves_installed_fds() {
    let parent = FileTable::new();
    let reservation = FileDescReservation::new(&parent, 4).unwrap();
    let installed = file();
    let installed_fd = parent
        .write()
        .insert_ceil_with_limit(installed.clone(), None, FdFlags::CLOEXEC, 4)
        .ok()
        .unwrap();
    let child = FileTable::fork_from(&parent.read());
    assert!(Arc::ptr_eq(
        child.read().get_file(installed_fd).unwrap(),
        &installed
    ));
    assert_eq!(
        child
            .write()
            .insert_ceil_with_limit(file(), None, FdFlags::empty(), 4)
            .ok(),
        Some(fd(0))
    );
    let closed = parent.write().close_files_on_exec();
    assert_eq!(closed.len(), 1);
    drop(closed);
    assert_eq!(reservation.install(file(), FdFlags::empty()), fd(0));
}
