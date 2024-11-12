// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use super::{process_filter::ProcessFilter, signal::constants::SIGCHLD, ExitCode, Pid, Process};
use crate::{
    prelude::*,
    process::{
        posix_thread::{thread_table, AsPosixThread},
        process_table,
        signal::with_signal_blocked,
    },
};

// The definition of WaitOptions is from Occlum
bitflags! {
    pub struct WaitOptions: u32 {
        const WNOHANG = 0x1;
        //Note: Below flags are not supported yet
        const WSTOPPED = 0x2; // Same as WUNTRACED
        const WEXITED = 0x4;
        const WCONTINUED = 0x8;
        const WNOWAIT = 0x01000000;
        const WNOTHREAD = 0x20000000;
        const WALL = 0x40000000;
        const WCLONE = 0x80000000;
    }
}

impl WaitOptions {
    pub fn supported(&self) -> bool {
        let unsupported_flags = WaitOptions::all() - WaitOptions::WNOHANG;
        !self.intersects(unsupported_flags)
    }
}

pub fn wait_child_exit(
    child_filter: ProcessFilter,
    wait_options: WaitOptions,
    ctx: &Context,
) -> Result<Option<Arc<Process>>> {
    let current = ctx.process;
    let zombie_child = with_signal_blocked(ctx, SIGCHLD.into(), || {
        current.children_wait_queue().pause_until(|| {
            let unwaited_children = current
                .children()
                .lock()
                .values()
                .filter(|child| match child_filter {
                    ProcessFilter::Any => true,
                    ProcessFilter::WithPid(pid) => child.pid() == pid,
                    ProcessFilter::WithPgid(pgid) => child.pgid() == pgid,
                })
                .cloned()
                .collect::<Vec<_>>();

            if unwaited_children.is_empty() {
                return Some(Err(Error::with_message(
                    Errno::ECHILD,
                    "the process has no child to wait",
                )));
            }

            // return immediately if we find a zombie child
            let zombie_child = unwaited_children.iter().find(|child| child.is_zombie());

            if let Some(zombie_child) = zombie_child {
                let zombie_pid = zombie_child.pid();
                if wait_options.contains(WaitOptions::WNOWAIT) {
                    // does not reap child, directly return
                    return Some(Ok(Some(zombie_child.clone())));
                } else {
                    reap_zombie_child(current, zombie_pid);
                    return Some(Ok(Some(zombie_child.clone())));
                }
            }

            if wait_options.contains(WaitOptions::WNOHANG) {
                return Some(Ok(None));
            }

            // wait
            None
        })
    })??;

    Ok(zombie_child)
}

/// Free zombie child with pid, returns the exit code of child process.
fn reap_zombie_child(process: &Process, pid: Pid) -> ExitCode {
    let child_process = process.children().lock().remove(&pid).unwrap();
    assert!(child_process.is_zombie());
    for task in &*child_process.tasks().lock() {
        thread_table::remove_thread(task.as_posix_thread().unwrap().tid());
    }

    // Lock order: session table -> group table -> process table -> group of process
    // -> group inner -> session inner
    let mut session_table_mut = process_table::session_table_mut();
    let mut group_table_mut = process_table::group_table_mut();
    let mut process_table_mut = process_table::process_table_mut();

    let mut child_group_mut = child_process.process_group.lock();

    let process_group = child_group_mut.upgrade().unwrap();
    let mut group_inner = process_group.inner.lock();
    let session = group_inner.session.upgrade().unwrap();
    let mut session_inner = session.inner.lock();

    group_inner.remove_process(&child_process.pid());
    session_inner.remove_process(&child_process);
    *child_group_mut = Weak::new();

    if group_inner.is_empty() {
        group_table_mut.remove(&process_group.pgid());
        session_inner.remove_process_group(&process_group.pgid());

        if session_inner.is_empty() {
            session_table_mut.remove(&session.sid());
        }
    }

    process_table_mut.remove(child_process.pid());
    child_process.exit_code()
}
