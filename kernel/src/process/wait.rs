// SPDX-License-Identifier: MPL-2.0

use super::{
    process_filter::ProcessFilter,
    signal::{constants::SIGCHLD, with_sigmask_changed},
    ExitCode, Pid, Process,
};
use crate::{
    prelude::*,
    process::{
        posix_thread::{thread_table, AsPosixThread, ThreadWaitStatus},
        process_table,
    },
    thread::{AsThread, Thread},
    time::clocks::ProfClock,
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
    pub fn is_all_supported(&self) -> bool {
        let supported_args = WaitOptions::WNOHANG
            | WaitOptions::WSTOPPED
            | WaitOptions::WCONTINUED
            | WaitOptions::WNOWAIT;
        supported_args.contains(*self)
    }
}

pub fn do_wait(
    child_filter: ProcessFilter,
    wait_options: WaitOptions,
    ctx: &Context,
) -> Result<Option<WaitStatus>> {
    if !wait_options.is_all_supported() {
        warn!("unsupported wait options is found: {:?}", wait_options);
    }

    let zombie_child = with_sigmask_changed(
        ctx,
        |sigmask| sigmask + SIGCHLD,
        || {
            ctx.process.children_wait_queue().pause_until(|| {
                // Acquire the children lock initially to prevent race conditions.
                // We want to ensure that multiple waiting threads
                // do not return the same waited process/thread status.
                let mut children_lock = ctx.process.children().lock();

                let unwaited_children = children_lock
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

                if let Some(zombie_child) =
                    wait_child_zombie(&unwaited_children, wait_options, &mut children_lock)
                {
                    return Some(Ok(Some(zombie_child)));
                }

                if let Some(status) =
                    wait_thread(&unwaited_children, wait_options, &mut children_lock)
                {
                    return Some(Ok(Some(status)));
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

pub enum WaitStatus {
    Process(Arc<Process>),
    Thread(Arc<Thread>, ThreadWaitStatus),
}

impl WaitStatus {
    pub fn id(&self) -> u32 {
        match self {
            WaitStatus::Process(process) => process.pid(),
            WaitStatus::Thread(thread, _) => thread.as_posix_thread().unwrap().tid(),
        }
    }

    pub fn exit_status(&self) -> u32 {
        match self {
            WaitStatus::Process(process) => process.status().exit_code(),
            WaitStatus::Thread(_, thread_wait_status) => thread_wait_status.as_u32(),
        }
    }

    pub fn prof_clock(&self) -> &Arc<ProfClock> {
        match self {
            WaitStatus::Process(process) => process.prof_clock(),
            WaitStatus::Thread(thread, _) => thread.as_posix_thread().unwrap().prof_clock(),
        }
    }
}

fn wait_child_zombie(
    unwaited_children: &[Arc<Process>],
    wait_options: WaitOptions,
    children_lock: &mut BTreeMap<Pid, Arc<Process>>,
) -> Option<WaitStatus> {
    let zombie_child = unwaited_children
        .iter()
        .find(|child| child.status().is_zombie())?;

    let wait_status = WaitStatus::Process(zombie_child.clone());
    if wait_options.contains(WaitOptions::WNOWAIT) {
        Some(wait_status)
    } else {
        let zombie_pid = zombie_child.pid();
        reap_zombie_child(zombie_pid, children_lock);
        Some(wait_status)
    }
}

fn wait_thread(
    unwaited_children: &[Arc<Process>],
    wait_options: WaitOptions,
    _children_lock: &mut BTreeMap<Pid, Arc<Process>>,
) -> Option<WaitStatus> {
    // Lock order: Children of process -> Tasks of process ->
    for process in unwaited_children.iter() {
        for task in process.tasks().lock().as_slice() {
            let posix_thread = task.as_posix_thread().unwrap();
            if let Some(thread_status) = posix_thread.status().lock().wait(wait_options) {
                return Some(WaitStatus::Thread(
                    task.as_thread().unwrap().clone(),
                    thread_status,
                ));
            }
        }
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

    // Lock order: children of process -> session table -> group table -> process table -> group of process
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
