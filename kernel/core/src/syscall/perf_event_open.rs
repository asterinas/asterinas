// SPDX-License-Identifier: MPL-2.0

//! System call handler for `perf_event_open(2)`.
//!
//! Implements the counting subset of the Linux `perf_event_open` ABI for
//! hardware CPU cycles and retired instructions.
//!
//! Reference: <https://man7.org/linux/man-pages/man2/perf_event_open.2.html>.

use ostd::{
    cpu::{self, CpuId},
    pmu::{self, Event, PmuError},
};

mod attributes;

use super::SyscallReturn;
use crate::{
    events::perf::{PerfContext, PerfEventFile, policy, to_error},
    fs::file::file_table::{FdFlags, FileTableInsert},
    prelude::*,
    process::{
        pid_table,
        posix_thread::{AsPosixThread, alien_access::AlienAccessMode},
    },
    thread::{Thread, Tid},
};
const PERF_FLAG_FD_CLOEXEC: u64 = 1 << 3;

/// Opens a standalone hardware counting event on the specified thread.
///
/// The returned file descriptor refers to the event in the target's [`PerfContext`].
/// An explicit `cpu` includes only the target's execution on that CPU.
/// Attaching to a running target may wait for its CPU to configure the event.
pub(super) fn sys_perf_event_open(
    attr_addr: Vaddr,
    pid: i32,
    cpu: i32,
    group_fd: i32,
    flags: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if flags & !PERF_FLAG_FD_CLOEXEC != 0 || pid < -1 || cpu < -1 || group_fd < -1 {
        return_errno_with_message!(Errno::EINVAL, "invalid perf_event_open arguments");
    }
    if pid == -1 && cpu == -1 {
        return_errno_with_message!(
            Errno::EINVAL,
            "system-wide perf_event_open requires a valid CPU"
        );
    }
    if pid == -1 || group_fd != -1 {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "system-wide events and event groups are unsupported"
        );
    }

    let target_cpu =
        if cpu == -1 {
            None
        } else {
            Some(CpuId::try_from(cpu as usize).map_err(|_| {
                Error::with_message(Errno::EINVAL, "PMU CPU number is out of range")
            })?)
        };

    let (spec, read_format) = attributes::read_counting(attr_addr, ctx, target_cpu)?;
    policy::check_open(&spec.config, ctx)?;
    let target = authorize_target(pid, ctx)?;
    check_cpu_capabilities(target_cpu, spec.config.event())?;
    let event_file = Arc::new(PerfEventFile::new(
        target.context,
        target.tid,
        spec,
        target.access_epoch,
        read_format,
    )?);

    let fd_flags = if flags & PERF_FLAG_FD_CLOEXEC != 0 {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };
    // The calling thread owns a live file table throughout the syscall.
    let file_table = ctx.thread_local.borrow_file_table();
    let fd = file_table.unwrap().lock_and_insert(event_file, fd_flags)?;
    Ok(SyscallReturn::Return(fd.into()))
}

struct AuthorizedTarget {
    context: Arc<PerfContext>,
    tid: Tid,
    access_epoch: u64,
}

fn authorize_target(pid: i32, ctx: &Context) -> Result<AuthorizedTarget> {
    if pid == 0 {
        let context = perf_context(ctx.thread)?;
        return Ok(AuthorizedTarget {
            context: context.clone(),
            tid: ctx.posix_thread.tid(),
            access_epoch: access_epoch(context)?,
        });
    }
    let thread = pid_table::pid_table_mut()
        .get_thread(pid as u32)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the PMU target thread does not exist"))?;
    let target = thread
        .as_posix_thread()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the PMU target is not a POSIX thread"))?;
    let context = perf_context(&thread)?;
    // Snapshot before checking credentials. Attachment verifies this epoch
    // under the same lock used by credential changes.
    let access_epoch = access_epoch(context)?;
    if !Weak::ptr_eq(target.weak_process(), ctx.posix_thread.weak_process()) {
        // The LSM hooks may access the target process after the PID table
        // lock is released. Keep it alive throughout the check.
        let _target_process = target.weak_process().upgrade().ok_or_else(|| {
            Error::with_message(Errno::ESRCH, "the PMU target process has exited")
        })?;
        target.check_alien_access_from(ctx.posix_thread, AlienAccessMode::READ_WITH_REAL_CREDS)?;
    }
    Ok(AuthorizedTarget {
        context: context.clone(),
        tid: pid as u32,
        access_epoch,
    })
}

fn perf_context(thread: &Thread) -> Result<&Arc<PerfContext>> {
    thread.perf().ok_or_else(|| to_error(PmuError::Unsupported))
}

fn access_epoch(context: &PerfContext) -> Result<u64> {
    context.access_epoch().ok_or_else(|| {
        Error::with_message(Errno::EACCES, "PMU target credentials cannot be authorized")
    })
}

fn check_cpu_capabilities(target_cpu: Option<CpuId>, event: Event) -> Result<()> {
    let check = |cpu| {
        let caps = pmu::capabilities(cpu).map_err(to_error)?;
        if !caps.supports_event(event) {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "PMU event is unavailable on a target CPU"
            );
        }
        Ok(())
    };
    if let Some(cpu) = target_cpu {
        return check(cpu);
    }
    cpu::all_cpus().try_for_each(check)
}
