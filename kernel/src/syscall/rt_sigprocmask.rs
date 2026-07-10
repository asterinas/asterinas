// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{posix_thread::ContextPthreadAdminApi, signal::sig_mask::SigSet},
};

pub fn sys_rt_sigprocmask(
    how: u32,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mask_op = MaskOp::try_from(how)?;
    debug!(
        "mask op = {:?}, set_ptr = 0x{:x}, oldset_ptr = 0x{:x}, sigset_size = {}",
        mask_op, set_ptr, oldset_ptr, sigset_size
    );

    let size_policy = RequireFullSize::new(sigset_size)?;
    do_rt_sigprocmask(mask_op, set_ptr, oldset_ptr, size_policy, ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn do_rt_sigprocmask(
    mask_op: MaskOp,
    set_ptr: Vaddr,
    oldset_ptr: Vaddr,
    size_policy: RequireFullSize,
    ctx: &Context,
) -> Result<()> {
    let old_sig_mask_value = ctx.posix_thread.sig_mask();
    debug!("old sig mask value: 0x{:x}", old_sig_mask_value);
    let user_space = ctx.user_space();

    let oldset_ptr = UserSigSetPtr::new(&user_space, oldset_ptr, size_policy);
    if oldset_ptr.addr() != 0 {
        oldset_ptr.write_val(&old_sig_mask_value)?;
    }

    let set_ptr = UserSigSetPtr::new(&user_space, set_ptr, size_policy);
    if set_ptr.addr() != 0 {
        let read_mask = set_ptr.read_val()?;
        match mask_op {
            MaskOp::Block => {
                ctx.set_sig_mask(old_sig_mask_value + read_mask);
            }
            MaskOp::Unblock => {
                ctx.set_sig_mask(old_sig_mask_value - read_mask);
            }
            MaskOp::SetMask => {
                ctx.set_sig_mask(read_mask);
            }
        }
    }
    debug!("new set = {:x?}", ctx.posix_thread.sig_mask());

    Ok(())
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum MaskOp {
    Block = 0,
    Unblock = 1,
    SetMask = 2,
}

/// A user-space pointer to a [`SigSet`] bound to a size policy `P`,
/// which governs how strict the `sigsetsize` argument must be.
pub(super) struct UserSigSetPtr<'a, 'b, P> {
    user_space: &'a CurrentUserSpace<'b>,
    addr: Vaddr,
    size_policy: P,
}

impl<'a, 'b, P> UserSigSetPtr<'a, 'b, P> {
    pub(super) fn new(user_space: &'a CurrentUserSpace<'b>, addr: Vaddr, size_policy: P) -> Self {
        Self {
            user_space,
            addr,
            size_policy,
        }
    }

    /// Returns the user-space address this pointer refers to.
    pub(super) fn addr(&self) -> Vaddr {
        self.addr
    }
}

/// Size policy requiring the full size of [`SigSet`].
///
/// Used by syscalls whose Linux ABI demands that `sigsetsize`
/// be exactly `size_of::<SigSet>()`.
#[derive(Clone, Copy)]
pub(super) struct RequireFullSize;

impl RequireFullSize {
    /// Validates that `size` is exactly `size_of::<SigSet>()`.
    pub(super) fn new(size: usize) -> Result<Self> {
        if size != size_of::<SigSet>() {
            return_errno_with_message!(Errno::EINVAL, "the sigset size is invalid");
        }
        Ok(RequireFullSize)
    }
}

/// Size policy allowing a [`SigSet`] to be written in a truncated form.
///
/// Used by syscalls `rt_sigpending`, whose Linux ABI rejects only sizes
/// greater than `size_of::<SigSet>()` and copies at most the requested number of bytes.
#[derive(Clone, Copy)]
pub(super) struct AllowTruncSize {
    real_size: usize,
}

impl AllowTruncSize {
    /// Validates that `size` is not greater than `size_of::<SigSet>()`.
    pub(super) fn new(size: usize) -> Result<Self> {
        if size > size_of::<SigSet>() {
            return_errno_with_message!(Errno::EINVAL, "the sigset size is too large");
        }
        Ok(AllowTruncSize { real_size: size })
    }
}

impl<'a, 'b> UserSigSetPtr<'a, 'b, RequireFullSize> {
    /// Reads a full [`SigSet`] from user space.
    pub(super) fn read_val(&self) -> Result<SigSet> {
        let val: u64 = self.user_space.read_val(self.addr)?;
        Ok(SigSet::from(val))
    }

    /// Writes a full [`SigSet`] to user space.
    pub(super) fn write_val(&self, sigset: &SigSet) -> Result<()> {
        self.user_space.write_val(self.addr, &u64::from(*sigset))?;
        Ok(())
    }
}

impl<'a, 'b> UserSigSetPtr<'a, 'b, AllowTruncSize> {
    /// Writes a possibly truncated [`SigSet`] to user space.
    ///
    /// Only the first `sigsetsize` bytes (stored in the policy) are written.
    pub(super) fn write_val(&self, sigset: &SigSet) -> Result<()> {
        let bytes = u64::from(*sigset).to_ne_bytes();
        self.user_space
            .write_bytes(self.addr, &bytes[..self.size_policy.real_size])?;
        Ok(())
    }
}
