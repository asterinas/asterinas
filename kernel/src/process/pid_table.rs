// SPDX-License-Identifier: MPL-2.0

use super::{Pgid, Pid, PidNamespace, Process, ProcessGroup, Session, Sid};
use crate::{
    events::{Events, Observer, Subject},
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::{AsThread, Thread, Tid},
};

static PID_TABLE: Mutex<PidTable> = Mutex::new(PidTable::new());

/// Transitional wrapper around namespace-visible PID tables.
///
/// This wrapper no longer owns a global object table. It only coordinates
/// root-namespace lookups and a small process-exit observer list.
pub(crate) struct PidTable {
    subject: Subject<PidEvent>,
}

impl PidTable {
    const fn new() -> Self {
        Self {
            subject: Subject::new(),
        }
    }

    pub fn insert_thread(&mut self, thread: Arc<Thread>) {
        let posix_thread = thread.as_posix_thread().unwrap();
        let tid_chain = posix_thread.tid_chain().clone();
        for link in tid_chain.links() {
            link.ns().insert_thread_chain(&thread, &tid_chain);
        }
    }

    pub fn remove_thread(&mut self, tid: Tid) {
        let Some(thread) = self.get_thread(tid) else {
            return;
        };

        let tid_chain = thread.as_posix_thread().unwrap().tid_chain().clone();
        for link in tid_chain.links() {
            link.ns().remove_thread_chain(&tid_chain);
        }
    }

    pub fn get_thread(&self, tid: Tid) -> Option<Arc<Thread>> {
        PidNamespace::get_init_singleton().lookup_thread(tid)
    }

    pub fn iter_threads(&self) -> impl Iterator<Item = Arc<Thread>> {
        PidNamespace::get_init_singleton()
            .visible_threads()
            .into_iter()
    }

    pub fn insert_process(&mut self, process: Arc<Process>) {
        for link in process.pid_chain().links() {
            link.ns().insert_process_chain(&process);
        }
    }

    pub fn remove_process(&mut self, pid: Pid) {
        let root_ns = PidNamespace::get_init_singleton();
        let Some(process) = root_ns.lookup_process(pid) else {
            return;
        };

        for link in process.pid_chain().links() {
            link.ns().remove_process_chain(&process);
        }

        if let Some(root_pid) = process.pid_in(root_ns) {
            self.subject.notify_observers(&PidEvent::Exit(root_pid));
        }
    }

    pub fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        PidNamespace::get_init_singleton().lookup_process(pid)
    }

    pub fn iter_processes(&self) -> impl Iterator<Item = Arc<Process>> {
        PidNamespace::get_init_singleton()
            .visible_processes()
            .into_iter()
    }

    pub fn process_count(&self) -> usize {
        PidNamespace::get_init_singleton().visible_process_count()
    }

    pub fn insert_process_group(&mut self, pgid: Pgid, group: Arc<ProcessGroup>) {
        let _ = pgid;
        for link in group.pgid_chain().links() {
            link.ns()
                .insert_process_group_chain(&group, group.pgid_chain());
        }
    }

    pub fn remove_process_group(&mut self, pgid: Pgid) {
        let root_ns = PidNamespace::get_init_singleton();
        let Some(group) = root_ns.lookup_process_group(pgid) else {
            return;
        };

        for link in group.pgid_chain().links() {
            link.ns().remove_process_group_chain(group.pgid_chain());
        }
    }

    pub fn get_process_group(&self, pgid: &Pgid) -> Option<Arc<ProcessGroup>> {
        PidNamespace::get_init_singleton().lookup_process_group(*pgid)
    }

    pub fn contains_process_group(&self, pgid: Pgid) -> bool {
        PidNamespace::get_init_singleton().contains_process_group(pgid)
    }

    pub fn insert_session(&mut self, sid: Sid, session: Arc<Session>) {
        let _ = sid;
        for link in session.sid_chain().links() {
            link.ns()
                .insert_session_chain(&session, session.sid_chain());
        }
    }

    pub fn remove_session(&mut self, sid: Sid) {
        let root_ns = PidNamespace::get_init_singleton();
        let Some(session) = root_ns.lookup_session(sid) else {
            return;
        };

        for link in session.sid_chain().links() {
            link.ns().remove_session_chain(session.sid_chain());
        }
    }

    pub fn register_observer(&mut self, observer: Weak<dyn Observer<PidEvent>>) {
        self.subject.register_observer(observer);
    }
}

pub(crate) fn pid_table_mut() -> MutexGuard<'static, PidTable> {
    PID_TABLE.lock()
}

pub(in crate::process) fn make_current_main_thread(ctx: &Context) {
    let pid = ctx.process.pid();
    let old_tid = ctx.posix_thread.tid();

    if old_tid == pid {
        return;
    }

    let pid_table = pid_table_mut();
    let mut tasks = ctx.process.tasks().lock();

    assert!(tasks.has_exited_main());
    assert!(tasks.in_execve());
    assert_eq!(tasks.as_slice().len(), 2);
    assert!(core::ptr::eq(ctx.task, tasks.as_slice()[1].as_ref()));

    tasks.swap_main(pid, old_tid);

    let old_tid_chain = ctx.posix_thread.tid_chain().clone();
    let new_tid_chain = ctx.process.pid_chain().clone();
    let thread = ctx.task.as_thread().unwrap().clone();

    for link in old_tid_chain.links() {
        link.ns().remove_thread_chain(&old_tid_chain);
    }

    ctx.posix_thread.set_main(new_tid_chain.clone());

    for link in new_tid_chain.links() {
        link.ns().insert_thread_chain(&thread, &new_tid_chain);
    }

    drop(pid_table);
}

#[derive(Copy, Clone)]
pub(crate) enum PidEvent {
    Exit(Pid),
}

impl Events for PidEvent {}
