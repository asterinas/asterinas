// SPDX-License-Identifier: MPL-2.0

//! System call handlers for `io_uring`.

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::file::{
        FileLike,
        file_table::{FdFlags, RawFileDesc, get_file_fast},
    },
    io_uring::{IoUringContext, IoUringEnterFlags, IoUringParams, IoUringSetupConfig},
    prelude::*,
    process::{posix_thread::ContextPthreadAdminApi, signal::sig_mask::SigMask},
};

pub fn sys_io_uring_setup(
    entries: u32,
    params_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut params = ctx.user_space().read_val::<IoUringParams>(params_addr)?;
    debug!("entries = {}, params = {:?}", entries, params);

    let setup_config = IoUringSetupConfig::new(entries, &params)?;
    let ring = IoUringContext::new(&setup_config, ctx)?;

    setup_config.write_params(&mut params);
    ctx.user_space().write_val(params_addr, &params)?;

    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let fd = file_table_locked.insert(ring, FdFlags::CLOEXEC);

    Ok(SyscallReturn::Return(fd.into()))
}

pub fn sys_io_uring_enter(
    raw_fd: RawFileDesc,
    to_submit: u32,
    min_complete: u32,
    flags: u32,
    sig_addr: Vaddr,
    sigsz: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let enter_flags = IoUringEnterFlags::from_user_bits(flags)?;
    debug!(
        "raw_fd = {}, to_submit = {}, min_complete = {}, flags = {:?}, sig_addr = 0x{:x}, sigsz = {}",
        raw_fd, to_submit, min_complete, enter_flags, sig_addr, sigsz
    );

    let ring_file = get_ring_file(raw_fd, ctx)?;
    let ring = ring_context(&ring_file)?;
    let submitted = if ring.is_sqpoll_mode() {
        if enter_flags.contains(IoUringEnterFlags::SQ_WAKEUP) {
            ring.wake_sqpoll_thread()?;
        }
        if enter_flags.contains(IoUringEnterFlags::SQ_WAIT) {
            ring.wait_for_sq_space()?;
        }

        to_submit
    } else {
        ring.submit_sqes(to_submit)?
    };

    if enter_flags.contains(IoUringEnterFlags::GETEVENTS) && min_complete > 0 {
        if sig_addr != 0 {
            // `IORING_ENTER_EXT_ARG` and `IORING_ENTER_EXT_ARG_REG` are rejected by
            // `IoUringEnterFlags::from_user_bits`, so `sig_addr` only interpreted as
            // the legacy `sigset_t *` argument here.
            let sigmask = ctx.user_space().read_val::<SigMask>(sig_addr)?;
            ctx.save_and_set_sig_mask(sigmask);
        }
        ring.wait_for_completions(min_complete)?;
    }

    Ok(SyscallReturn::Return(submitted as _))
}

pub fn sys_io_uring_register(
    raw_fd: RawFileDesc,
    opcode: u32,
    arg_addr: Vaddr,
    nr_args: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let ring_file = get_ring_file(raw_fd, ctx)?;
    let ring = ring_context(&ring_file)?;
    debug!(
        "raw_fd = {}, opcode = {}, nr_args = {}",
        raw_fd, opcode, nr_args
    );

    ring.register(opcode, arg_addr, nr_args)?;

    Ok(SyscallReturn::Return(0))
}

fn get_ring_file(raw_fd: RawFileDesc, ctx: &Context) -> Result<Arc<dyn FileLike>> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?).into_owned();

    if file.as_ref().downcast_ref::<IoUringContext>().is_none() {
        return_errno_with_message!(Errno::EINVAL, "the FD is not an io_uring file");
    }

    Ok(file)
}

fn ring_context(file: &Arc<dyn FileLike>) -> Result<&IoUringContext> {
    file.as_ref()
        .downcast_ref::<IoUringContext>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the FD is not an io_uring file"))
}
