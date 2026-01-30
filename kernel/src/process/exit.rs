// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::{Pid, Process, process_table};
use crate::{
    events::IoEvents, fs::cgroupfs::CgroupMembership, prelude::*,
    process::signal::signals::kernel::KernelSignal,
};

/// Exits the current POSIX process.
///
/// This is for internal use. Do NOT call this directly. When the last thread in the process exits,
/// [`do_exit`] or [`do_exit_group`] will invoke this method automatically.
///
/// [`do_exit`]: crate::process::posix_thread::do_exit
/// [`do_exit_group`]: crate::process::posix_thread::do_exit_group
pub(super) fn exit_process(current_process: &Process) {
    current_process.status().set_zombie();
    current_process.status().set_vfork_child(false);

    // Drop fields in `Process`.
    current_process.lock_vmar().set_vmar(None);

    current_process.pidfile_pollee.notify(IoEvents::IN);

    send_parent_death_signal(current_process);

    move_children_to_reaper_process(current_process);

    send_child_death_signal(current_process);

    // Remove the process from the cgroup.
    let mut cgroup_guard = CgroupMembership::lock();
    cgroup_guard.move_process_to_root(current_process);
    drop(cgroup_guard);
}

/// Sends parent-death signals to the children.
//
// FIXME: According to the Linux implementation, the signal should be sent when the POSIX thread
// that created the child exits, not when the whole process exits. For more details, see the
// "CAVEATS" section in <https://man7.org/linux/man-pages/man2/pr_set_pdeathsig.2const.html>.
fn send_parent_death_signal(current_process: &Process) {
    let current_children = current_process.children().lock();
    for child in current_children.as_ref().unwrap().values() {
        let Some(signum) = child.parent_death_signal() else {
            continue;
        };

        // FIXME: Set `si_pid` in the `siginfo_t` argument.
        let signal = Box::new(KernelSignal::new(signum));
        child.enqueue_signal(signal);
    }
}

/// Finds a reaper process for `current_process`.
///
/// If there is no reaper process for `current_process`, returns `None`.
fn find_reaper_process(current_process: &Process) -> Option<Arc<Process>> {
    let mut parent = current_process.parent().lock().process().upgrade().unwrap();

    loop {
        if parent.is_init_process() {
            return Some(parent);
        }

        if !parent.has_child_subreaper.load(Ordering::Acquire) {
            return None;
        }

        let is_reaper = parent.is_child_subreaper();
        let is_zombie = parent.status().is_zombie();
        if is_reaper && !is_zombie {
            return Some(parent);
        }

        let grandparent = parent.parent().lock().process().upgrade();
        if let Some(grandparent) = grandparent {
            parent = grandparent;
        } else {
            // If both the parent and grandparent have exited concurrently, we will lose the clue
            // about the ancestor processes. Therefore, we have to retry.
            parent = current_process.parent().lock().process().upgrade().unwrap();
        }
    }
}

/// Moves the children of `current_process` to be the children of `reaper_process`.
///
/// If the `reaper_process` is zombie, returns `Err(())`.
fn move_process_children(
    current_process: &Process,
    reaper_process: &Arc<Process>,
) -> core::result::Result<(), ()> {
    // Lock order: children of process -> parent of process
    let mut reaper_process_children = reaper_process.children().lock();
    let Some(reaper_process_children) = reaper_process_children.as_mut() else {
        // The reaper process has exited, and it is not the init process
        // (since we never clear the init process's children).
        return Err(());
    };

    // We hold the lock of children while updating the children's parents.
    // This ensures when dealing with CLONE_PARENT,
    // the retrial will see an up-to-date real parent.
    let mut current_children = current_process.children().lock();
    for child_process in current_children.as_mut().unwrap().values() {
        let mut parent = child_process.parent.lock();
        reaper_process_children.insert(child_process.pid(), child_process.clone());
        parent.set_process(reaper_process);
    }
    *current_children = None;

    Ok(())
}

/// Moves the children to a reaper process.
fn move_children_to_reaper_process(current_process: &Process) {
    if current_process.is_init_process() {
        return;
    }

    while let Some(reaper_process) = find_reaper_process(current_process) {
        if move_process_children(current_process, &reaper_process).is_ok() {
            reaper_process.children_wait_queue().wake_all();
            return;
        }
    }

    const INIT_PROCESS_PID: Pid = 1;

    let init_process = process_table::get_process(INIT_PROCESS_PID).unwrap();
    move_process_children(current_process, &init_process).unwrap();
    init_process.children_wait_queue().wake_all();
}

/// Sends a child-death signal to the parent.
fn send_child_death_signal(current_process: &Process) {
    let Some(parent) = current_process.parent().lock().process().upgrade() else {
        return;
    };

    if let Some(signal) = current_process.exit_signal().map(KernelSignal::new) {
        parent.enqueue_signal(Box::new(signal));
    };
    parent.children_wait_queue().wake_all();
}
