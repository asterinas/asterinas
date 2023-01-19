//! This mod defines mmap flags and the handler to syscall mmap

use crate::fs::file::FileDescripter;
use crate::process::process_vm::mmap_flags::MMapFlags;
use crate::rights::Rights;
use crate::vm::perms::VmPerms;
use crate::vm::vmo::VmoOptions;
use crate::{log_syscall_entry, prelude::*};
use jinux_frame::vm::VmPerm;

use crate::syscall::SYS_MMAP;

use super::SyscallReturn;

pub fn sys_mmap(
    addr: u64,
    len: u64,
    perms: u64,
    flags: u64,
    fd: u64,
    offset: u64,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_MMAP);
    let perms = VmPerm::try_from(perms).unwrap();
    let flags = MMapFlags::try_from(flags).unwrap();
    let res = do_sys_mmap(
        addr as usize,
        len as usize,
        perms,
        flags,
        fd as _,
        offset as usize,
    )?;
    Ok(SyscallReturn::Return(res as _))
}

pub fn do_sys_mmap(
    addr: Vaddr,
    len: usize,
    vm_perm: VmPerm,
    flags: MMapFlags,
    fd: FileDescripter,
    offset: usize,
) -> Result<Vaddr> {
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, perms = {:?}, flags = {:?}, fd = {}, offset = 0x{:x}",
        addr, len, vm_perm, flags, fd, offset
    );

    if flags.contains(MMapFlags::MAP_ANONYMOUS) {
        // only support map anonymous areas.
        mmap_anonymous_vmo(len, offset, vm_perm, flags)
    } else {
        panic!("Unsupported mmap flags: {:?}", flags);
    }
}

pub fn mmap_anonymous_vmo(
    len: usize,
    offset: usize,
    vm_perm: VmPerm,
    flags: MMapFlags,
) -> Result<Vaddr> {
    // TODO: how to respect flags?
    if flags.complement().contains(MMapFlags::MAP_ANONYMOUS)
        | flags.complement().contains(MMapFlags::MAP_PRIVATE)
    {
        panic!("Unsupported mmap flags {:?} now", flags);
    }

    if len % PAGE_SIZE != 0 {
        panic!("Mmap only support page-aligned len");
    }
    if offset % PAGE_SIZE != 0 {
        panic!("Mmap only support page-aligned offset");
    }
    let vmo_options: VmoOptions<Rights> = VmoOptions::new(len);
    let vmo = vmo_options.alloc()?;
    let current = current!();
    let root_vmar = current.root_vmar();
    let perms = VmPerms::from(vm_perm);
    let mut vmar_map_options = root_vmar.new_map(vmo, perms)?;
    if flags.contains(MMapFlags::MAP_FIXED) {
        vmar_map_options = vmar_map_options.offset(offset);
    }
    Ok(vmar_map_options.build()?)
}
