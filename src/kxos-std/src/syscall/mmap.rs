//! This mod defines mmap flags and the handler to syscall mmap

use crate::prelude::*;
use crate::process::process_vm::mmap_area::MMapFlags;
use kxos_frame::vm::VmPerm;

use crate::{process::Process, syscall::SYS_MMAP};

use super::SyscallReturn;

pub fn sys_mmap(
    addr: u64,
    len: u64,
    perms: u64,
    flags: u64,
    fd: u64,
    offset: u64,
) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_MMAP]", SYS_MMAP);
    let perms = VmPerm::try_from(perms).unwrap();
    let flags = MMapFlags::try_from(flags).unwrap();
    let res = do_sys_mmap(
        addr as usize,
        len as usize,
        perms,
        flags,
        fd as usize,
        offset as usize,
    );
    Ok(SyscallReturn::Return(res as _))
}

pub fn do_sys_mmap(
    addr: Vaddr,
    len: usize,
    vm_perm: VmPerm,
    flags: MMapFlags,
    fd: usize,
    offset: usize,
) -> Vaddr {
    debug!("addr = 0x{:x}", addr);
    debug!("len = {}", len);
    debug!("perms = {:?}", vm_perm);
    debug!("flags = {:?}", flags);
    debug!("fd = 0x{:x}", fd);
    debug!("offset = 0x{:x}", offset);

    if flags.contains(MMapFlags::MAP_ANONYMOUS) & !flags.contains(MMapFlags::MAP_FIXED) {
        // only support map anonymous areas on **NOT** fixed addr now
    } else {
        panic!("Unsupported mmap flags: {:?}", flags);
    }

    let current = Process::current();
    let mmap_area = current
        .mmap_area()
        .expect("mmap should work on process with mmap area");
    let vm_space = current
        .vm_space()
        .expect("mmap should work on process with user space");
    // current.mmap(len, vm_perm, flags, offset)
    mmap_area.mmap(len, offset, vm_perm, flags, vm_space)
}
