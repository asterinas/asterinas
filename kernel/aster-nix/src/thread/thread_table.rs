// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::BTreeMap, sync::Arc};

use ostd::{sync::SpinLock, task::Task};

use super::{SharedThreadInfo, ThreadExt, Tid};

lazy_static::lazy_static! {
    static ref THREAD_TABLE: SpinLock<BTreeMap<Tid, Arc<Task>>> = SpinLock::new(BTreeMap::new());
}

pub fn add_thread(thread: Arc<Task>) {
    let tid = thread.thread_info().unwrap().tid;
    (*THREAD_TABLE).lock().insert(tid, thread);
}

pub fn remove_thread(tid: Tid) {
    (*THREAD_TABLE).lock().remove(&tid);
}

pub fn get_thread(tid: Tid) -> Option<Arc<Task>> {
    (*THREAD_TABLE).lock().get(&tid).cloned()
}
