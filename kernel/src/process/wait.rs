// SPDX-License-Identifier: MPL-2.0

use super::{
    ExitCode, Pid, Process,
    posix_thread::AsPosixThread,
    process_filter::ProcessFilter,
    signal::{constants::SIGCHLD, with_sigmask_changed},
};
use crate::{
    prelude::*,
    process::{
        KernelPid, PidNamespace, ReapedChildrenStats, Uid, namespace::pid_ns::pid_ns_graph_lock,
        signal::sig_num::SigNum, status::StopWaitStatus,
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
                let _pid_ns_graph_guard = pid_ns_graph_lock().lock();
                // Acquire the children lock at first to prevent race conditions.
                // We want to ensure that multiple waiting threads
                // do not return the same waited process status.
                let mut children_lock = ctx.process.children().lock();
                let children_mut = children_lock.as_mut().unwrap();
                let caller_pid_ns = ctx.process.active_pid_ns();

                let unwaited_children = children_mut
                    .values()
                    .filter(|child| match &child_filter {
                        ProcessFilter::Any => true,
                        ProcessFilter::WithPid(pid) => child.pid_in(caller_pid_ns) == Some(*pid),
                        ProcessFilter::WithPgid(pgid) => {
                            child.pgid_in(caller_pid_ns) == Some(*pgid)
                        }
                        ProcessFilter::WithPidfd(pid_file) => match pid_file.process_opt() {
                            Some(process) => Arc::ptr_eq(&process, child),
                            None => false,
                        },
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
                        let child_process = children_mut.remove(&status.kernel_pid()).unwrap();
                        drop(children_lock);
                        reap_zombie_child(child_process, ctx.process.reaped_children_stats());
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
    pub fn pid_in(&self, ns: &crate::process::PidNamespace) -> Option<Pid> {
        self.process().pid_in(ns)
    }

    fn kernel_pid(&self) -> KernelPid {
        self.process().kernel_pid()
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

/// Frees a zombie child and returns the exit code of the child process.
fn reap_zombie_child(
    child_process: Arc<Process>,
    reaped_children_stats: &Mutex<ReapedChildrenStats>,
) -> ExitCode {
    assert!(child_process.status().is_zombie());

    let tid_chains = child_process
        .tasks()
        .lock()
        .as_slice()
        .iter()
        .map(|task| task.as_posix_thread().unwrap().tid_chain().clone())
        .collect::<Vec<_>>();

    for tid_chain in &tid_chains {
        PidNamespace::remove_thread_across_namespaces_with_pid_ns_graph_lock(tid_chain);
    }
    PidNamespace::remove_process_across_namespaces_with_pid_ns_graph_lock(child_process.as_ref());

    let mut child_group_mut = child_process.process_group.lock();
    child_process.clear_old_group_and_session_with_pid_ns_graph_lock(&mut child_group_mut);

    let (mut user_time, mut kernel_time) = child_process.reaped_children_stats().lock().get();
    user_time += child_process.prof_clock().user_clock().read_time();
    kernel_time += child_process.prof_clock().kernel_clock().read_time();
    reaped_children_stats.lock().add(user_time, kernel_time);

    child_process.status().exit_code()
}
