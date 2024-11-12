// SPDX-License-Identifier: MPL-2.0

use super::{Thread, Tid};
use crate::{prelude::*, process::posix_thread::AsPosixThread};

static THREAD_TABLE: SpinLock<BTreeMap<Tid, Arc<Thread>>> = SpinLock::new(BTreeMap::new());

/// Adds a posix thread to global thread table
pub fn add_thread(tid: Tid, thread: Arc<Thread>) {
    debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());
    THREAD_TABLE.lock().insert(tid, thread);
}

/// Removes a posix thread to global thread table
pub fn remove_thread(tid: Tid) {
    THREAD_TABLE.lock().remove(&tid);
}

/// Gets a posix thread from the global thread table
pub fn get_thread(tid: Tid) -> Option<Arc<Thread>> {
    THREAD_TABLE.lock().get(&tid).cloned()
}
