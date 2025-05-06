// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use super::{
    process_filter::ProcessFilter,
    signal::{constants::SIGCHLD, with_sigmask_changed},
    ExitCode, Pid, Process,
};
use crate::{
    prelude::*,
    process::{
        posix_thread::{thread_table, AsPosixThread},
        process_table,
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
    let zombie_child = with_sigmask_changed(
        ctx,
        |sigmask| sigmask + SIGCHLD,
        || {
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
                let zombie_child = unwaited_children
                    .iter()
                    .find(|child| child.status().is_zombie());

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
        },
    )??;

    Ok(zombie_child)
}

/// Free zombie child with pid, returns the exit code of child process.
fn reap_zombie_child(process: &Process, pid: Pid) -> ExitCode {
    let child_process = process.children().lock().remove(&pid).unwrap();
    assert!(child_process.status().is_zombie());

    for task in child_process.tasks().lock().as_slice() {
        thread_table::remove_thread(task.as_posix_thread().unwrap().tid());
    }

    // Lock order: session table -> group table -> process table -> group of process
    // -> group inner -> session inner
    let mut session_table_mut = process_table::session_table_mut();
    let mut group_table_mut = process_table::group_table_mut();

    // Remove the process from the global table
    let mut process_table_mut = process_table::process_table_mut();
    process_table_mut.remove(child_process.pid());

    // Remove the process group and the session from global table, if necessary
    let mut child_group_mut = child_process.process_group.lock();
    child_process.clear_old_group_and_session(
        &mut child_group_mut,
        &mut session_table_mut,
        &mut group_table_mut,
    );
    *child_group_mut = Weak::new();

    child_process.status().exit_code()
}
