// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FdFlags,
        inode_handle::InodeHandle,
        ramfs::memfd::{MAX_MEMFD_NAME_LEN, MemfdFlags, MemfdInodeHandle},
    },
    prelude::*,
};

pub fn sys_memfd_create(name_addr: Vaddr, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    // FIXME: When `name` is too long, `read_cstring` returns `EFAULT`. However,
    // according to <https://man7.org/linux/man-pages/man2/memfd_create.2.html>,
    // we should return `EINVAL` in this case.
    let name = ctx
        .user_space()
        .read_cstring(name_addr, MAX_MEMFD_NAME_LEN + 1)?;
    debug!("sys_memfd_create: name = {:?}, flags = {}", name, flags);

    let fd = {
        let memfd_flags = MemfdFlags::from_bits(flags).ok_or(Errno::EINVAL)?;
        let fd_flags = if memfd_flags.contains(MemfdFlags::MFD_CLOEXEC) {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();

        // FIXME: Support `MFD_HUGETLB`.
        if memfd_flags.contains(MemfdFlags::MFD_HUGETLB) {
            warn!("`MFD_HUGETLB` not supported");
        }

        if memfd_flags.contains(MemfdFlags::MFD_NOEXEC_SEAL | MemfdFlags::MFD_EXEC) {
            return_errno_with_message!(
                Errno::EINVAL,
                "`MFD_EXEC` and `MFD_NOEXEC_SEAL` cannot be specified together"
            );
        }

        let memfd_file = InodeHandle::new_memfd(name.to_string_lossy().into_owned(), memfd_flags)?;

        file_table_locked.insert(Arc::new(memfd_file), fd_flags)
    };

    Ok(SyscallReturn::Return(fd as _))
}
