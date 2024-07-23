// SPDX-License-Identifier: MPL-2.0

use super::{process_table, Pid, Process, TermStatus};
use crate::{
    prelude::*,
    process::{
        posix_thread::do_exit,
        signal::{constants::SIGCHLD, signals::kernel::KernelSignal},
    },
};

pub fn do_exit_group(term_status: TermStatus) {
    let current = current!();
    debug!("exit group was called");
    if current.is_zombie() {
        return;
    }
    current.set_zombie(term_status);

    // Exit all threads
    let threads = current.threads().lock().clone();
    for thread in threads {
        if let Err(e) = do_exit(thread, term_status) {
            debug!("Ignore error when call exit: {:?}", e);
        }
    }

    // Sends parent-death signal
    // FIXME: according to linux spec, the signal should be sent when a posix thread which
    // creates child process exits, not when the whole process exits group.
    for (_, child) in current.children().lock().iter() {
        let Some(signum) = child.parent_death_signal() else {
            continue;
        };

        // FIXME: set pid of the signal
        let signal = KernelSignal::new(signum);
        child.enqueue_signal(signal);
    }

    // Close all files then exit the process
    let files = current.file_table().lock().close_all();
    drop(files);

    // Move children to the init process
    if !is_init_process(&current) {
        if let Some(init_process) = get_init_process() {
            let mut init_children = init_process.children().lock();
            for (_, child_process) in current.children().lock().extract_if(|_, _| true) {
                let mut parent = child_process.parent.lock();
                init_children.insert(child_process.pid(), child_process.clone());
                *parent = Arc::downgrade(&init_process);
            }
        }
    }

    if let Some(parent) = current.parent() {
        // Notify parent
        let signal = KernelSignal::new(SIGCHLD);
        parent.enqueue_signal(signal);
        parent.children_pauser().resume_all();
    }
}

const INIT_PROCESS_PID: Pid = 1;

/// Gets the init process
fn get_init_process() -> Option<Arc<Process>> {
    process_table::get_process(INIT_PROCESS_PID)
}

fn is_init_process(process: &Process) -> bool {
    process.pid() == INIT_PROCESS_PID
}
