// SPDX-License-Identifier: MPL-2.0

use super::{posix_thread::ThreadLocal, process_table, Pid, Process};
use crate::{prelude::*, process::signal::signals::kernel::KernelSignal};

/// Exits the current POSIX process.
///
/// This is for internal use. Do NOT call this directly. When the last thread in the process exits,
/// [`do_exit`] or [`do_exit_group`] will invoke this method automatically.
///
/// [`do_exit`]: crate::process::posix_thread::do_exit
/// [`do_exit_group`]: crate::process::posix_thread::do_exit_group
pub(super) fn exit_process(thread_local: &ThreadLocal, current_process: &Process) {
    current_process.status().set_zombie();

    if current_process.status().is_vfork() {
        current_process.status().set_vfork_status(false);
    }
    // FIXME: This is obviously wrong in a number of ways, since different threads can have
    // different file tables, and different processes can share the same file table.
    thread_local.file_table().borrow().write().close_all();

    send_parent_death_signal(current_process);

    move_children_to_init(current_process);

    send_child_death_signal(current_process);

    current_process.root_vmar().clear();
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

/// Moves the children to the init process.
fn move_children_to_init(current_process: &Process) {
    if is_init_process(current_process) {
        return;
    }

    let Some(init_process) = get_init_process() else {
        return;
    };

    let mut init_children = init_process.children().lock();
    for (_, child_process) in current_process.children().lock().extract_if(|_, _| true) {
        let mut parent = child_process.parent.lock();
        init_children.insert(child_process.pid(), child_process.clone());
        parent.set_process(&init_process);
    }
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
