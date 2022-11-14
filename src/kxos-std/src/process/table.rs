//! A global table stores the pid to process mapping.
//! This table can be used to get process with pid.
//! TODO: progress group, thread all need similar mapping

use crate::prelude::*;

use super::{process_group::ProcessGroup, Pgid, Pid, Process};

lazy_static! {
    static ref PROCESS_TABLE: Mutex<BTreeMap<Pid, Arc<Process>>> = Mutex::new(BTreeMap::new());
    static ref PROCESS_GROUP_TABLE: Mutex<BTreeMap<Pgid, Arc<ProcessGroup>>> =
        Mutex::new(BTreeMap::new());
}

/// add a process to global table
pub fn add_process(process: Arc<Process>) {
    let pid = process.pid();
    PROCESS_TABLE.lock().insert(pid, process);
}

/// remove a process from global table
pub fn remove_process(pid: Pid) {
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

/// add process group to global table
pub fn add_process_group(process_group: Arc<ProcessGroup>) {
    let pgid = process_group.pgid();
    PROCESS_GROUP_TABLE.lock().insert(pgid, process_group);
}

/// remove process group from global table
pub fn remove_process_group(pgid: Pgid) {
    PROCESS_GROUP_TABLE.lock().remove(&pgid);
}

/// get a process group with pgid
pub fn pgid_to_process_group(pgid: Pgid) -> Option<Arc<ProcessGroup>> {
    PROCESS_GROUP_TABLE
        .lock()
        .get(&pgid)
        .map(|process_group| process_group.clone())
}
