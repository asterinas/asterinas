// SPDX-License-Identifier: MPL-2.0

//! Memory Protection Keys (pkeys) support.
//!
//! See <https://man7.org/linux/man-pages/man7/pkeys.7.html> for more details.

use super::SyscallReturn;
use crate::{
    prelude::*,
    vm::vmar::{PKey, PKeyAccessRights},
};

pub fn sys_pkey_alloc(flags: u32, access_rights: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("flags: {:?}, access_rights: {:?}", flags, access_rights);

    // Flags is reserved for future use in Linux, see the man pages for details.
    if flags != 0 {
        return_errno_with_message!(Errno::EINVAL, "flags is reserved for future use");
    }
    let Some(rights) = PKeyAccessRights::from_bits(access_rights) else {
        return_errno_with_message!(Errno::EINVAL, "access_rights contains invalid bits");
    };

    let uspace = ctx.user_space();
    let vmar = uspace.root_vmar();

    vmar.alloc_pkey(rights).map(|pkey| {
        debug!("allocated pkey: {:?}", pkey);
        SyscallReturn::Return(pkey as isize)
    })
}

pub fn sys_pkey_free(pkey: PKey, ctx: &Context) -> Result<SyscallReturn> {
    debug!("pkey: {:?}", pkey);

    let uspace = ctx.user_space();
    let vmar = uspace.root_vmar();

    vmar.free_pkey(pkey).map(|_| {
        debug!("freed pkey: {:?}", pkey);
        SyscallReturn::NoReturn
    })
}

pub fn sys_pkey_mprotect(
    addr: Vaddr,
    len: usize,
    prot: u64,
    pkey: PKey,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "addr: {:?}, len: {:?}, pkey: {:?}, prot: {:?}",
        addr, len, pkey, prot
    );

    super::mprotect::do_sys_mprotect(addr, len, prot, pkey, ctx)
}
