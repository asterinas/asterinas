// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

use super::SyscallReturn;
use crate::{
    fs::file_table::FdFlags,
    prelude::*,
    vm::memfd::{MAX_MEMFD_NAME_LEN, MEMFD_MANAGER},
};

bitflags! {
    struct MemfdFlags: u32 {
        /// close on exec
        const MFD_CLOEXEC = 1 << 0;
        /// allow sealing operations on this file
        const MFD_ALLOW_SEALING = 1 << 1;
        /// create in the hugetlbfs
        const MFD_HUGETLB = 1 << 2;
    }
}

pub fn sys_memfd_create(name_addr: Vaddr, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    // FIXME: when `name` is too long, `read_cstring` returns `EFAULT`. However,
    // according to <https://man7.org/linux/man-pages/man2/memfd_create.2.html>,
    // we should return `EINVAL` in this case.
    let name = ctx
        .user_space()
        .read_cstring(name_addr, MAX_MEMFD_NAME_LEN + 1)?;
    debug!("sys_memfd_create: name = {:?}, flags = {}", name, flags);

    let file_handle = {
        let inode_handle = MEMFD_MANAGER
            .get()
            .ok_or(Errno::EINVAL)?
            .create(name.to_string_lossy().as_ref())?;
        Arc::new(inode_handle)
    };

    let fd = {
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();
        let memfd_flags = MemfdFlags::from_bits(flags).ok_or(Errno::EINVAL)?;
        let fd_flags = if memfd_flags.contains(MemfdFlags::MFD_CLOEXEC) {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };
        // FIXME: Support `MFD_ALLOW_SEALING` and `MFD_HUGETLB`.
        if memfd_flags.contains(MemfdFlags::MFD_ALLOW_SEALING) {
            warn!("sealing not supported");
        }
        file_table_locked.insert(file_handle.clone(), fd_flags)
    };

    Ok(SyscallReturn::Return(fd as _))
}
