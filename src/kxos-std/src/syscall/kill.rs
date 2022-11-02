use crate::prelude::*;

use crate::process::signal::signals::user::{UserSignal, UserSignalKind};
use crate::process::{table, Process};
use crate::{
    process::{process_filter::ProcessFilter, signal::sig_num::SigNum},
    syscall::SYS_KILL,
};

pub fn sys_kill(process_filter: u64, sig_num: u64) -> Result<isize> {
    debug!("[syscall][id={}][SYS_KILL]", SYS_KILL);
    let process_filter = ProcessFilter::from_id(process_filter as _);
    let sig_num = SigNum::try_from(sig_num as u8).unwrap();
    do_sys_kill(process_filter, sig_num)?;
    Ok(0)
}

pub fn do_sys_kill(process_filter: ProcessFilter, sig_num: SigNum) -> Result<()> {
    let current = current!();
    let pid = current.pid();
    // FIXME: use the correct uid
    let uid = 0;
    let processes = get_processes(&process_filter)?;
    for process in processes.iter() {
        if process.status().lock().is_zombie() {
            continue;
        }

        let signal = Box::new(UserSignal::new(sig_num, UserSignalKind::Kill, pid, uid));
        let sig_queues = process.sig_queues();
        sig_queues.lock().enqueue(signal);
    }
    Ok(())
}

fn get_processes(filter: &ProcessFilter) -> Result<Vec<Arc<Process>>> {
    let processes = match filter {
        ProcessFilter::Any => {
            let mut processes = table::get_all_processes();
            processes.retain(|process| process.pid() != 0);
            processes
        }
        ProcessFilter::WithPid(pid) => {
            let process = table::pid_to_process(*pid);
            match process {
                None => return_errno!(Errno::ESRCH),
                Some(process) => vec![process],
            }
        }
        ProcessFilter::WithPgid(_) => todo!(),
    };
    Ok(processes)
}
