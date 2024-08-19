// SPDX-License-Identifier: MPL-2.0

use super::{Thread, Tid};
use crate::prelude::*;

lazy_static! {
    static ref THREAD_TABLE: Mutex<BTreeMap<Tid, Arc<Thread>>> = Mutex::new(BTreeMap::new());
}

pub fn add_thread(thread: Arc<Thread>) {
    let tid = thread.tid();
    THREAD_TABLE.lock().insert(tid, thread);
}

pub fn remove_thread(tid: Tid) {
    THREAD_TABLE.lock().remove(&tid);
}

pub fn get_thread(tid: Tid) -> Option<Arc<Thread>> {
    THREAD_TABLE.lock().get(&tid).cloned()
}
