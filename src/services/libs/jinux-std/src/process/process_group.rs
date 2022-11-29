use super::{table, Pgid, Pid, Process};
use crate::prelude::*;

pub struct ProcessGroup {
    inner: Mutex<ProcessGroupInner>,
}

struct ProcessGroupInner {
    pgid: Pgid,
    processes: BTreeMap<Pid, Arc<Process>>,
    leader_process: Option<Arc<Process>>,
}

impl ProcessGroup {
    fn default() -> Self {
        ProcessGroup {
            inner: Mutex::new(ProcessGroupInner {
                pgid: 0,
                processes: BTreeMap::new(),
                leader_process: None,
            }),
        }
    }

    pub fn new(process: Arc<Process>) -> Self {
        let process_group = ProcessGroup::default();
        let pid = process.pid();
        process_group.set_pgid(pid);
        process_group.add_process(process.clone());
        process_group.set_leader_process(process);
        process_group
    }

    pub fn set_pgid(&self, pgid: Pgid) {
        self.inner.lock().pgid = pgid;
    }

    pub fn set_leader_process(&self, leader_process: Arc<Process>) {
        self.inner.lock().leader_process = Some(leader_process);
    }

    pub fn add_process(&self, process: Arc<Process>) {
        self.inner.lock().processes.insert(process.pid(), process);
    }

    /// remove a process from this process group.
    /// If this group contains no processes now, the group itself will be deleted from global table.
    pub fn remove_process(&self, pid: Pid) {
        let mut inner_lock = self.inner.lock();
        inner_lock.processes.remove(&pid);
        let len = inner_lock.processes.len();
        let pgid = inner_lock.pgid;
        // if self contains no process, remove self from table
        if len == 0 {
            // this must be the last statement
            table::remove_process_group(pgid);
        }
    }

    pub fn pgid(&self) -> Pgid {
        self.inner.lock().pgid
    }
}
