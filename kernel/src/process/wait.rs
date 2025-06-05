// SPDX-License-Identifier: MPL-2.0

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
        signal::sig_num::SigNum,
        status::StopWaitStatus,
        Uid,
    },
    time::clocks::ProfClock,
};

// The definition of WaitOptions is from Occlum
bitflags! {
    pub struct WaitOptions: u32 {
        const WNOHANG = 0x1;
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
    pub fn check(&self) -> Result<()> {
        // FIXME: The syscall `waitid` allows using WNOWAIT with
        // WSTOPPED or WCONTINUED
        if self.intersects(WaitOptions::WSTOPPED | WaitOptions::WCONTINUED)
            && self.contains(WaitOptions::WNOWAIT)
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "WNOWAIT cannot be used toghther with WSTOPPED or WCONTINUED"
            );
        }

        let supported_args = WaitOptions::WNOHANG
            | WaitOptions::WSTOPPED
            | WaitOptions::WCONTINUED
            | WaitOptions::WNOWAIT;
        if !supported_args.contains(*self) {
            warn!(
                "unsupported wait options are found: {:?}",
                *self - supported_args
            );
        }

        Ok(())
    }
}

pub fn do_wait(
    child_filter: ProcessFilter,
    wait_options: WaitOptions,
    ctx: &Context,
) -> Result<Option<WaitStatus>> {
    wait_options.check()?;

    let is_nonblocking = if let ProcessFilter::WithPidfd(pid_file) = &child_filter {
        pid_file.is_nonblocking()
    } else {
        false
    };

    let zombie_child = with_sigmask_changed(
        ctx,
        |sigmask| sigmask + SIGCHLD,
        || {
            ctx.process.children_wait_queue().pause_until(|| {
                // Acquire the children lock at first to prevent race conditions.
                // We want to ensure that multiple waiting threads
                // do not return the same waited process status.
                let mut children_lock = ctx.process.children().lock();

                let unwaited_children = children_lock
                    .values()
                    .filter(|child| match &child_filter {
                        ProcessFilter::Any => true,
                        ProcessFilter::WithPid(pid) => child.pid() == *pid,
                        ProcessFilter::WithPgid(pgid) => child.pgid() == *pgid,
                        ProcessFilter::WithPidfd(pid_file) => {
                            Arc::ptr_eq(pid_file.process(), *child)
                        }
                    })
                    .collect::<Box<_>>();

                if unwaited_children.is_empty() {
                    return Some(Err(Error::with_message(
                        Errno::ECHILD,
                        "the process has no child to wait",
                    )));
                }

                if let Some(status) = wait_zombie(&unwaited_children) {
                    if !wait_options.contains(WaitOptions::WNOWAIT) {
                        reap_zombie_child(status.pid(), &mut children_lock);
                    }
                    return Some(Ok(Some(status)));
                }

                if let Some(status) = wait_stopped_or_continued(&unwaited_children, wait_options) {
                    return Some(Ok(Some(status)));
                }

                if wait_options.contains(WaitOptions::WNOHANG) {
                    return Some(Ok(None));
                }

                if is_nonblocking {
                    return Some(Err(Error::with_message(
                        Errno::EAGAIN,
                        "the PID file is nonblocking and the child has not terminated",
                    )));
                }

                // wait
                None
            })
        },
    )??;

    Ok(zombie_child)
}

pub enum WaitStatus {
    Zombie(Arc<Process>),
    Stop(Arc<Process>, SigNum),
    Continue(Arc<Process>),
}

impl WaitStatus {
    pub fn pid(&self) -> Pid {
        self.process().pid()
    }

    pub fn uid(&self) -> Uid {
        self.process()
            .main_thread()
            .as_posix_thread()
            .unwrap()
            .credentials()
            .ruid()
    }

    pub fn prof_clock(&self) -> &Arc<ProfClock> {
        self.process().prof_clock()
    }

    fn process(&self) -> &Arc<Process> {
        match self {
            WaitStatus::Zombie(process)
            | WaitStatus::Stop(process, _)
            | WaitStatus::Continue(process) => process,
        }
    }
}

fn wait_zombie(unwaited_children: &[&Arc<Process>]) -> Option<WaitStatus> {
    unwaited_children
        .iter()
        .find(|child| child.status().is_zombie())
        .map(|child| WaitStatus::Zombie((*child).clone()))
}

fn wait_stopped_or_continued(
    unwaited_children: &[&Arc<Process>],
    wait_options: WaitOptions,
) -> Option<WaitStatus> {
    if !wait_options.intersects(WaitOptions::WSTOPPED | WaitOptions::WCONTINUED) {
        return None;
    }

    // Lock order: children of process -> tasks of process
    for process in unwaited_children.iter() {
        let Some(stop_wait_status) = process.wait_stopped_or_continued(wait_options) else {
            continue;
        };

        let wait_status = match stop_wait_status {
            StopWaitStatus::Stopped(sig_num) => WaitStatus::Stop((*process).clone(), sig_num),
            StopWaitStatus::Continue => WaitStatus::Continue((*process).clone()),
        };
        return Some(wait_status);
    }

    None
}

/// Free zombie child with pid, returns the exit code of child process.
fn reap_zombie_child(pid: Pid, children_lock: &mut BTreeMap<Pid, Arc<Process>>) -> ExitCode {
    let child_process = children_lock.remove(&pid).unwrap();
    assert!(child_process.status().is_zombie());

    for task in child_process.tasks().lock().as_slice() {
        thread_table::remove_thread(task.as_posix_thread().unwrap().tid());
    }

    // Lock order: children of process -> session table -> group table
    // -> process table -> group of process -> group inner -> session inner
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
