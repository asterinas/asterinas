// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::{SyscallReturn, SYS_MUNMAP};
use crate::{log_syscall_entry, prelude::*};

pub fn sys_munmap(addr: Vaddr, len: usize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_MUNMAP);
    debug!("addr = 0x{:x}, len = {}", addr, len);
    let current = current!();
    let root_vmar = current.root_vmar();
    let len = len.align_up(PAGE_SIZE);
    debug!("unmap range = 0x{:x} - 0x{:x}", addr, addr + len);
    root_vmar.destroy(addr..addr + len)?;
    Ok(SyscallReturn::Return(0))
}
