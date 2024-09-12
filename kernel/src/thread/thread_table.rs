// SPDX-License-Identifier: MPL-2.0

use keyable_arc::KeyableArc;

use super::{Thread, Tid};
use crate::{prelude::*, process::posix_thread::PosixThreadExt};

static POSIX_THREAD_TABLE: SpinLock<BTreeMap<Tid, Arc<Thread>>> = SpinLock::new(BTreeMap::new());
static KERNEL_THREAD_TABLE: SpinLock<BTreeSet<KeyableArc<Thread>>> = SpinLock::new(BTreeSet::new());

pub fn add_posix_thread(tid: Tid, thread: Arc<Thread>) {
    debug_assert_eq!(tid, thread.tid());
    POSIX_THREAD_TABLE.lock().insert(tid, thread);
}

pub fn remove_posix_thread(tid: Tid) {
    POSIX_THREAD_TABLE.lock().remove(&tid);
}

pub fn get_posix_thread(tid: Tid) -> Option<Arc<Thread>> {
    POSIX_THREAD_TABLE.lock().get(&tid).cloned()
}

pub(super) fn add_kernel_thread(thread: Arc<Thread>) {
    KERNEL_THREAD_TABLE.lock().insert(KeyableArc::from(thread));
}

pub(super) fn remove_kernel_thread(thread: Arc<Thread>) {
    KERNEL_THREAD_TABLE.lock().remove(&KeyableArc::from(thread));
}
