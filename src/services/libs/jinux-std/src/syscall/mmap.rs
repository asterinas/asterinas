//! This mod defines mmap flags and the handler to syscall mmap

use crate::fs::file_table::FileDescripter;
use crate::process::process_vm::mmap_flags::MMapFlags;
use crate::rights::Rights;
use crate::vm::perms::VmPerms;
use crate::vm::vmo::{VmoChildOptions, VmoOptions, VmoRightsOp};
use crate::{log_syscall_entry, prelude::*};
use jinux_frame::vm::VmPerm;
use jinux_frame::AlignExt;

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

    let len = len.align_up(PAGE_SIZE);

    if len % PAGE_SIZE != 0 {
        panic!("Mmap only support page-aligned len");
    }
    if offset % PAGE_SIZE != 0 {
        panic!("Mmap only support page-aligned offset");
    }
    let perms = VmPerms::from(vm_perm);

    if flags.contains(MMapFlags::MAP_ANONYMOUS) {
        // only support map anonymous areas.
        mmap_anonymous_vmo(addr, len, offset, perms, flags)
    } else {
        mmap_filebacked_vmo(addr, fd, len, offset, perms, flags)
    }
}

fn mmap_anonymous_vmo(
    addr: Vaddr,
    len: usize,
    offset: usize,
    perms: VmPerms,
    flags: MMapFlags,
) -> Result<Vaddr> {
    // TODO: how to respect flags?
    if flags.complement().contains(MMapFlags::MAP_ANONYMOUS)
        | flags.complement().contains(MMapFlags::MAP_PRIVATE)
    {
        panic!("Unsupported mmap flags {:?} now", flags);
    }
    debug_assert!(offset == 0);

    let vmo_options: VmoOptions<Rights> = VmoOptions::new(len);
    let vmo = vmo_options.alloc()?;
    let current = current!();
    let root_vmar = current.root_vmar();

    let mut vmar_map_options = root_vmar.new_map(vmo, perms)?;
    if flags.contains(MMapFlags::MAP_FIXED) {
        vmar_map_options = vmar_map_options.offset(addr).can_overwrite(true);
    }
    let map_addr = vmar_map_options.build()?;
    debug!("map addr = 0x{:x}", map_addr);
    Ok(map_addr)
}

fn mmap_filebacked_vmo(
    addr: Vaddr,
    fd: FileDescripter,
    len: usize,
    offset: usize,
    perms: VmPerms,
    flags: MMapFlags,
) -> Result<Vaddr> {
    let current = current!();
    let fs_resolver = current.fs().read();
    let dentry = fs_resolver.lookup_from_fd(fd)?;
    let vnode = dentry.vnode();
    let page_cache_vmo = vnode.page_cache();

    let vmo = if flags.contains(MMapFlags::MAP_PRIVATE) {
        // map private
        VmoChildOptions::new_cow(page_cache_vmo, offset..(offset + len)).alloc()?
    } else {
        // map shared
        // FIXME: enable slice child to exceed parent range
        VmoChildOptions::new_slice(page_cache_vmo, offset..(offset + len)).alloc()?
    };

    let root_vmar = current.root_vmar();
    let mut vm_map_options = root_vmar.new_map(vmo.to_dyn(), perms)?;
    if flags.contains(MMapFlags::MAP_FIXED) {
        vm_map_options = vm_map_options.offset(addr).can_overwrite(true);
    }
    let map_addr = vm_map_options.build()?;
    debug!("map addr = 0x{:x}", map_addr);
    Ok(map_addr)
}
