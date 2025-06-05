// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::{process_table, Pid, Process};
use crate::{events::IoEvents, prelude::*, process::signal::signals::kernel::KernelSignal};

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
    current_process.lock_root_vmar().set_vmar(None);

    current_process.pidfile_pollee.notify(IoEvents::IN);

    send_parent_death_signal(current_process);

    move_children_to_reaper_process(current_process);

    send_child_death_signal(current_process);
}

/// Sends parent-death signals to the children.
//
// FIXME: According to the Linux implementation, the signal should be sent when the POSIX thread
// that created the child exits, not when the whole process exits. For more details, see the
// "CAVEATS" section in <https://man7.org/linux/man-pages/man2/pr_set_pdeathsig.2const.html>.
fn send_parent_death_signal(current_process: &Process) {
    for (_, child) in current_process.children().lock().iter() {
        let Some(signum) = child.parent_death_signal() else {
            continue;
        };

        // FIXME: Set `si_pid` in the `siginfo_t` argument.
        let signal = KernelSignal::new(signum);
        child.enqueue_signal(signal);
    }
}

/// Finds a reaper process for `current_process`.
///
/// If there is no reaper process for `current_process`, returns `None`.
fn find_reaper_process(current_process: &Process) -> Option<Arc<Process>> {
    let mut parent = current_process.parent().lock().process();

    while let Some(process) = parent.upgrade() {
        if is_init_process(&process) {
            return Some(process);
        }

        if !process.has_child_subreaper.load(Ordering::Acquire) {
            return None;
        }

        let is_reaper = process.is_child_subreaper();
        let is_zombie = process.status().is_zombie();
        if is_reaper && !is_zombie {
            return Some(process);
        }

        parent = process.parent().lock().process();
    }

    None
}

/// Moves the children of `current_process` to be the children of `reaper_process`.
///
/// If the `reaper_process` is zombie, returns `Err(())`.
fn move_process_children(
    current_process: &Process,
    reaper_process: &Arc<Process>,
) -> core::result::Result<(), ()> {
    // Take the lock first to avoid the race when the `reaper_process` is exiting concurrently.
    let mut reaper_process_children = reaper_process.children().lock();

    let is_init = is_init_process(reaper_process);
    let is_zombie = reaper_process.status().is_zombie();
    if !is_init && is_zombie {
        return Err(());
    }

    for (_, child_process) in current_process.children().lock().extract_if(|_, _| true) {
        let mut parent = child_process.parent.lock();
        reaper_process_children.insert(child_process.pid(), child_process.clone());
        parent.set_process(reaper_process);
    }
    Ok(())
}

/// Moves the children to a reaper process.
fn move_children_to_reaper_process(current_process: &Process) {
    if is_init_process(current_process) {
        return;
    }

    while let Some(reaper_process) = find_reaper_process(current_process) {
        if move_process_children(current_process, &reaper_process).is_ok() {
            return;
        }
    }

    let Some(init_process) = get_init_process() else {
        return;
    };

    let _ = move_process_children(current_process, &init_process);
}

/// Sends a child-death signal to the parent.
fn send_child_death_signal(current_process: &Process) {
    let Some(parent) = current_process.parent().lock().process().upgrade() else {
        return;
    };

    if let Some(signal) = current_process.exit_signal().map(KernelSignal::new) {
        parent.enqueue_signal(signal);
    };
    parent.children_wait_queue().wake_all();
}

const INIT_PROCESS_PID: Pid = 1;

/// Gets the init process
fn get_init_process() -> Option<Arc<Process>> {
    process_table::get_process(INIT_PROCESS_PID)
}

fn is_init_process(process: &Process) -> bool {
    process.pid() == INIT_PROCESS_PID
}
