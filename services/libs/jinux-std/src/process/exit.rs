use crate::process::posix_thread::PosixThreadExt;
use crate::process::signal::signals::kernel::KernelSignal;
use crate::{prelude::*, process::signal::constants::SIGCHLD};

use super::{process_table, Pid, Process, TermStatus};

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
        if thread.is_exited() {
            continue;
        }

        thread.exit();
        if let Some(posix_thread) = thread.as_posix_thread() {
            let tid = thread.tid();
            if let Err(e) = posix_thread.exit(tid, term_status) {
                debug!("Ignore error when call exit: {:?}", e);
            }
        }
    }

    // Close all files then exit the process
    let files = current.file_table().lock().close_all();
    for file in files {
        let _ = file.clean_for_close();
    }

    // Move children to the init process
    if !is_init_process(&current) {
        if let Some(init_process) = get_init_process() {
            for (_, child_process) in current.children().lock().extract_if(|_, _| true) {
                child_process.set_parent(Arc::downgrade(&init_process));
                init_process.add_child(child_process);
            }
        }
    }

    if let Some(parent) = current.parent() {
        // Notify parent
        let signal = Box::new(KernelSignal::new(SIGCHLD));
        parent.enqueue_signal(signal);
        parent.children_pauser().resume_all();
    }
}

const INIT_PROCESS_PID: Pid = 1;

/// Get the init process
fn get_init_process() -> Option<Arc<Process>> {
    process_table::pid_to_process(INIT_PROCESS_PID)
}

fn is_init_process(process: &Process) -> bool {
    process.pid() == INIT_PROCESS_PID
}
