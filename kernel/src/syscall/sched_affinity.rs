// SPDX-License-Identifier: MPL-2.0

use core::{cmp, mem};

use ostd::cpu::{num_cpus, CpuId, CpuSet};

use super::SyscallReturn;
use crate::{prelude::*, process::posix_thread::thread_table, thread::Tid};

pub fn sys_sched_getaffinity(
    tid: Tid,
    cpuset_size: usize,
    cpu_set_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let cpu_set = match tid {
        0 => ctx.thread.atomic_cpu_affinity().load(),
        _ => match thread_table::get_thread(tid) {
            Some(thread) => thread.atomic_cpu_affinity().load(),
            None => return Err(Error::with_message(Errno::ESRCH, "thread does not exist")),
        },
    };

    let bytes_written = write_cpu_set_to(ctx.user_space(), &cpu_set, cpuset_size, cpu_set_ptr)?;

    Ok(SyscallReturn::Return(bytes_written as isize))
}

// TODO: The manual page of `sched_setaffinity` says that if the thread is not
// running on the CPU specified in the affinity mask, it would be migrated to
// one of the CPUs specified in the mask. We currently do not support this
// feature as the scheduler is not ready for migration yet.
pub fn sys_sched_setaffinity(
    tid: Tid,
    cpuset_size: usize,
    cpu_set_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_cpu_set = read_cpu_set_from(ctx.user_space(), cpuset_size, cpu_set_ptr)?;

    match tid {
        0 => ctx.thread.atomic_cpu_affinity().store(&user_cpu_set),
        _ => match thread_table::get_thread(tid) {
            Some(thread) => {
                thread.atomic_cpu_affinity().store(&user_cpu_set);
            }
            None => return Err(Error::with_message(Errno::ESRCH, "thread does not exist")),
        },
    }

    Ok(SyscallReturn::Return(0))
}

// Linux uses `DECLARE_BITMAP` for `cpu_set_t`, inside which each part is a
// `long`. We use the same scheme to ensure byte endianness compatibility.
type Part = u64;
const SIZE_OF_PART: usize = mem::size_of::<Part>();
const CPUS_IN_PART: usize = SIZE_OF_PART * 8;

fn read_cpu_set_from(
    uspace: CurrentUserSpace,
    cpuset_size: usize,
    cpu_set_ptr: Vaddr,
) -> Result<CpuSet> {
    if cpuset_size == 0 {
        return Err(Error::with_message(Errno::EINVAL, "invalid cpuset size"));
    }

    let num_cpus = num_cpus();

    let mut ret_set = CpuSet::new_empty();

    let nr_parts_to_read = cmp::min(cpuset_size / SIZE_OF_PART, num_cpus.div_ceil(CPUS_IN_PART));
    for part_id in 0..nr_parts_to_read {
        let user_part: Part = uspace.read_val(cpu_set_ptr + part_id * SIZE_OF_PART)?;
        for bit_id in 0..CPUS_IN_PART {
            if user_part & (1 << bit_id) != 0 {
                // If the CPU ID is invalid, just ignore it.
                let Ok(cpu_id) = CpuId::try_from(part_id * CPUS_IN_PART + bit_id) else {
                    continue;
                };
                ret_set.add(cpu_id);
            }
        }
    }

    if ret_set.is_empty() {
        return Err(Error::with_message(Errno::EINVAL, "empty cpuset"));
    }

    Ok(ret_set)
}

// Returns the number of bytes written.
fn write_cpu_set_to(
    uspace: CurrentUserSpace,
    cpu_set: &CpuSet,
    cpuset_size: usize,
    cpu_set_ptr: Vaddr,
) -> Result<usize> {
    if cpuset_size == 0 {
        return Err(Error::with_message(Errno::EINVAL, "invalid cpuset size"));
    }

    let num_cpus = num_cpus();

    let nr_parts_to_write = cmp::min(cpuset_size / SIZE_OF_PART, num_cpus.div_ceil(CPUS_IN_PART));
    let mut user_part: Part = 0;
    let mut part_idx = 0;
    for cpu_id in cpu_set.iter() {
        let id = cpu_id.as_usize();
        while part_idx < cmp::min(id / CPUS_IN_PART, nr_parts_to_write) {
            uspace.write_val(cpu_set_ptr + part_idx * SIZE_OF_PART, &user_part)?;
            user_part = 0;
            part_idx += 1;
        }
        if part_idx >= nr_parts_to_write {
            break;
        }
        user_part |= 1 << (id % CPUS_IN_PART);
    }

    while part_idx < nr_parts_to_write {
        uspace.write_val(cpu_set_ptr + part_idx * SIZE_OF_PART, &user_part)?;
        user_part = 0;
        part_idx += 1;
    }

    Ok(nr_parts_to_write * SIZE_OF_PART)
}
