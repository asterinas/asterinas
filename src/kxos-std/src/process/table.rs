//! A global table stores the pid to process mapping.
//! This table can be used to get process with pid.
//! TODO: progress group, thread all need similar mapping

use crate::prelude::*;

use super::{Pid, Process};

lazy_static! {
    static ref PROCESS_TABLE: Mutex<BTreeMap<Pid, Arc<Process>>> = Mutex::new(BTreeMap::new());
}

/// add a process to global table
pub fn add_process(pid: Pid, process: Arc<Process>) {
    PROCESS_TABLE.lock().insert(pid, process);
}

/// delete a process from global table
pub fn delete_process(pid: Pid) {
    PROCESS_TABLE.lock().remove(&pid);
}

/// get a process with pid
pub fn pid_to_process(pid: Pid) -> Option<Arc<Process>> {
    PROCESS_TABLE
        .lock()
        .get(&pid)
        .map(|process| process.clone())
}

/// get all processes
pub fn get_all_processes() -> Vec<Arc<Process>> {
    PROCESS_TABLE
        .lock()
        .iter()
        .map(|(_, process)| process.clone())
        .collect()
}
