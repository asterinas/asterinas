use crate::prelude::*;

use super::{process_filter::ProcessFilter, ExitCode, Pid};

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
    let (pid, exit_code) = current.waiting_children().wait_until(|| {
        let children_lock = current.children().lock();
        let unwaited_children = children_lock
            .iter()
            .filter(|(pid, child)| match child_filter {
                ProcessFilter::Any => true,
                ProcessFilter::WithPid(pid) => child.pid() == pid,
                ProcessFilter::WithPgid(pgid) => child.pgid() == pgid,
            })
            .map(|(_, child)| child.clone())
            .collect::<Vec<_>>();
        // we need to drop the lock here, since reap child process need to acquire this lock again
        drop(children_lock);

        if unwaited_children.len() == 0 {
            return Err(kxos_frame::Error::NoChild);
        }

        // return immediately if we find a zombie child
        let zombie_child = unwaited_children
            .iter()
            .find(|child| child.status().lock().is_zombie());

        if let Some(zombie_child) = zombie_child {
            let zombie_pid = zombie_child.pid();
            let exit_code = zombie_child.exit_code();
            if wait_options.contains(WaitOptions::WNOWAIT) {
                // does not reap child, directly return
                return Ok(Some((zombie_pid, exit_code)));
            } else {
                let exit_code = current.reap_zombie_child(zombie_pid);
                return Ok(Some((zombie_pid, exit_code)));
            }
        }

        if wait_options.contains(WaitOptions::WNOHANG) {
            return Ok(Some((0, 0)));
        }

        // wait
        Ok(None)
    })?;

    Ok((pid, exit_code))
}
