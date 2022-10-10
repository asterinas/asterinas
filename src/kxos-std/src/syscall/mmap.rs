//! This mod defines mmap flags and the handler to syscall mmap

use bitflags::bitflags;
use kxos_frame::{
    debug,
    vm::{Vaddr, VmPerm},
};

use crate::{process::Process, syscall::SYS_MMAP};

use super::SyscallResult;

// The definition of MMapFlags is from occlum
bitflags! {
    pub struct MMapFlags : u32 {
        const MAP_FILE            = 0x0;
        const MAP_SHARED          = 0x1;
        const MAP_PRIVATE         = 0x2;
        const MAP_SHARED_VALIDATE = 0x3;
        const MAP_TYPE            = 0xf;
        const MAP_FIXED           = 0x10;
        const MAP_ANONYMOUS       = 0x20;
        const MAP_GROWSDOWN       = 0x100;
        const MAP_DENYWRITE       = 0x800;
        const MAP_EXECUTABLE      = 0x1000;
        const MAP_LOCKED          = 0x2000;
        const MAP_NORESERVE       = 0x4000;
        const MAP_POPULATE        = 0x8000;
        const MAP_NONBLOCK        = 0x10000;
        const MAP_STACK           = 0x20000;
        const MAP_HUGETLB         = 0x40000;
        const MAP_SYNC            = 0x80000;
        const MAP_FIXED_NOREPLACE = 0x100000;
    }
}

impl TryFrom<u64> for MMapFlags {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        MMapFlags::from_bits(value as u32).ok_or_else(|| "unknown mmap flags")
    }
}

pub fn sys_mmap(
    addr: u64,
    len: u64,
    perms: u64,
    flags: u64,
    fd: u64,
    offset: u64,
) -> SyscallResult {
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
    SyscallResult::Return(res as _)
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
