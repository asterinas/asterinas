use crate::{prelude::*, process::process_table, thread::thread_table};

use super::{process_filter::ProcessFilter, ExitCode, Pid, Process};

// The definition of WaitOptions is from Occlum
bitflags! {
    pub struct WaitOptions: u32 {
        const WNOHANG = 0x1;
        //Note: Below flags are not supported yet
        const WSTOPPED = 0x2; // Same as WUNTRACED
        const WEXITED = 0x4;
        const WCONTINUED = 0x8;
        const WNOWAIT = 0x01000000;
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
) -> Result<(Pid, ExitCode)> {
    let current = current!();
    let (pid, exit_code) = current.children_pauser().pause_until(|| {
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
        let zombie_child = unwaited_children.iter().find(|child| child.is_zombie());

        if let Some(zombie_child) = zombie_child {
            let zombie_pid = zombie_child.pid();
            let exit_code = zombie_child.exit_code().unwrap();
            if wait_options.contains(WaitOptions::WNOWAIT) {
                // does not reap child, directly return
                return Some(Ok((zombie_pid, exit_code)));
            } else {
                let exit_code = reap_zombie_child(&current, zombie_pid);
                return Some(Ok((zombie_pid, exit_code)));
            }
        }

        if wait_options.contains(WaitOptions::WNOHANG) {
            return Some(Ok((0, 0)));
        }

        // wait
        None
    })??;

    Ok((pid, exit_code as _))
}

/// Free zombie child with pid, returns the exit code of child process.
fn reap_zombie_child(process: &Process, pid: Pid) -> u32 {
    let child_process = process.children().lock().remove(&pid).unwrap();
    assert!(child_process.is_zombie());
    child_process.root_vmar().destroy_all().unwrap();
    for thread in &*child_process.threads().lock() {
        thread_table::remove_thread(thread.tid());
    }
    process_table::remove_process(child_process.pid());
    if let Some(process_group) = child_process.process_group() {
        process_group.remove_process(child_process.pid());
    }
    child_process.exit_code().unwrap()
}
