// SPDX-License-Identifier: MPL-2.0

use core::{cmp, mem};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{process_table, Pid},
    util::write_val_to_user,
};

fn get_num_cpus() -> usize {
    // TODO: Properly determine the number of available CPUs
    // This could be through a system configuration query.
    1
}

pub fn sys_sched_getaffinity(
    pid: Pid,
    cpuset_size: usize,
    cpu_set_ptr: Vaddr,
) -> Result<SyscallReturn> {
    let num_cpus = get_num_cpus();

    if cpuset_size < core::mem::size_of::<cpu_set_t>() {
        return Err(Error::with_message(Errno::EINVAL, "invalid cpuset size"));
    }

    match pid {
        0 => {
            // TODO: Get the current thread's CPU affinity
            // Placeholder for future implementation.
        }
        _ => {
            match process_table::get_process(pid) {
                Some(_process) => { /* Placeholder if process-specific logic needed */ }
                None => return Err(Error::with_message(Errno::ESRCH, "process does not exist")),
            }
        }
    }

    let dummy_cpu_set = cpu_set_t::new(num_cpus);

    write_val_to_user(cpu_set_ptr, &dummy_cpu_set)?;

    Ok(SyscallReturn::Return(0))
}

const CPU_SETSIZE: usize = 1024; // Max number of CPU bits.
const __NCPUBITS: usize = 8 * mem::size_of::<usize>();

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C, packed)]
struct cpu_set_t {
    __bits: [usize; CPU_SETSIZE / __NCPUBITS],
}

impl cpu_set_t {
    /// Creates a new cpu_set_t representing available CPUs.
    fn new(num_cpus: usize) -> Self {
        let mut bits = [0usize; CPU_SETSIZE / __NCPUBITS];

        for cpu in 0..cmp::min(num_cpus, CPU_SETSIZE) {
            bits[cpu / __NCPUBITS] |= 1 << (cpu % __NCPUBITS);
        }

        Self { __bits: bits }
    }
}
