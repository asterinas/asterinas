// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use super::{
    process_filter::ProcessFilter,
    signal::{constants::SIGCHLD, with_sigmask_changed},
    ExitCode, Pid, Process,
};
use crate::{
    prelude::*,
    process::{pid_namespace::MapsOfProcess, posix_thread::AsPosixThread},
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
                        ProcessFilter::WithPid(pid) => {
                            child.pid_in_ns(ctx.process.pid_namespace()).unwrap() == pid
                        }
                        ProcessFilter::WithPgid(pgid) => {
                            child.pgid_in_ns(ctx.process.pid_namespace()).unwrap() == pgid
                        }
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
                    let zombie_pid = zombie_child.pid_in_ns(current.pid_namespace()).unwrap();
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

    // Lock order: group_of_process -> task list
    let mut child_group_mut = child_process.process_group.lock();
    let mut maps_of_process =
        MapsOfProcess::get_maps_and_lock_task_list(&child_process, &mut child_group_mut);

    for task in child_process.tasks().lock().as_slice() {
        let Some(posix_thread) = task.as_posix_thread() else {
            continue;
        };

        // Only the main thread is still in the child_process's tasks.
        debug_assert_eq!(
            posix_thread.tid_in_ns(child_process.pid_namespace()),
            child_process.pid_in_ns(child_process.pid_namespace())
        );

        maps_of_process.detach_thread();
    }

    // Remove the process from the global table
    maps_of_process.detach_process();

    // Remove the process group and the session from global table, if necessary
    child_process.clear_old_group_and_session(&mut child_group_mut, &mut maps_of_process);
    *child_group_mut = Weak::new();

    child_process.status().exit_code()
}
