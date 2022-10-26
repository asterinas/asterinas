use crate::prelude::*;

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
    process_filter: ProcessFilter,
    wait_options: WaitOptions,
) -> (Pid, ExitCode) {
    let current = Process::current();

    let (pid, exit_code) = current.waiting_children().wait_until(process_filter, || {
        let waited_child_process = match process_filter {
            ProcessFilter::Any => current.get_zombie_child(),
            ProcessFilter::WithPid(pid) => current.get_child_by_pid(pid as Pid),
            ProcessFilter::WithPgid(pgid) => todo!(),
        };

        // some child process is exited
        if let Some(waited_child_process) = waited_child_process {
            let wait_pid = waited_child_process.pid();
            let exit_code = waited_child_process.exit_code();
            if wait_options.contains(WaitOptions::WNOWAIT) {
                // does not reap child, directly return
                return Some((wait_pid, exit_code));
            } else {
                let exit_code = current.reap_zombie_child(wait_pid);
                return Some((wait_pid, exit_code));
            }
        }

        if wait_options.contains(WaitOptions::WNOHANG) {
            return Some((0, 0));
        }

        None
    });

    (pid, exit_code)
}
