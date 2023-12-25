//! This mod defines mmap flags and the handler to syscall mmap

use crate::fs::file_table::FileDescripter;
use crate::vm::perms::VmPerms;
use crate::vm::vmo::{VmoChildOptions, VmoOptions, VmoRightsOp};
use crate::{log_syscall_entry, prelude::*};
use align_ext::AlignExt;
use aster_frame::vm::VmPerm;
use aster_rights::Rights;

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
    let option = MMapOptions::try_from(flags as u32).unwrap();
    let res = do_sys_mmap(
        addr as usize,
        len as usize,
        perms,
        option,
        fd as _,
        offset as usize,
    )?;
    Ok(SyscallReturn::Return(res as _))
}

pub fn do_sys_mmap(
    addr: Vaddr,
    len: usize,
    vm_perm: VmPerm,
    option: MMapOptions,
    fd: FileDescripter,
    offset: usize,
) -> Result<Vaddr> {
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, perms = {:?}, option = {:?}, fd = {}, offset = 0x{:x}",
        addr, len, vm_perm, option, fd, offset
    );

    let len = len.align_up(PAGE_SIZE);

    if offset % PAGE_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "mmap only support page-aligned offset");
    }
    let perms = VmPerms::from(vm_perm);

    if option.flags().contains(MMapFlags::MAP_ANONYMOUS) {
        mmap_anonymous_vmo(addr, len, offset, perms, option)
    } else {
        mmap_filebacked_vmo(addr, fd, len, offset, perms, option)
    }
}

fn mmap_anonymous_vmo(
    addr: Vaddr,
    len: usize,
    offset: usize,
    perms: VmPerms,
    option: MMapOptions,
) -> Result<Vaddr> {
    assert!(option.flags().contains(MMapFlags::MAP_ANONYMOUS));
    debug_assert!(offset == 0);

    // TODO: implement features presented by other flags.
    if option.typ() != MMapType::Private {
        panic!("Unsupported mmap flags {:?} now", option);
    }

    let vmo_options: VmoOptions<Rights> = VmoOptions::new(len);
    let vmo = vmo_options.alloc()?;
    let current = current!();
    let root_vmar = current.root_vmar();

    let mut vmar_map_options = root_vmar.new_map(vmo, perms)?;
    if option.flags().contains(MMapFlags::MAP_FIXED) {
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
    option: MMapOptions,
) -> Result<Vaddr> {
    let current = current!();
    let page_cache_vmo = {
        let fs_resolver = current.fs().read();
        let dentry = fs_resolver.lookup_from_fd(fd)?;
        let inode = dentry.inode();
        inode.page_cache().ok_or(Error::with_message(
            Errno::EBADF,
            "File does not have page cache",
        ))?
    };

    let vmo = if option.typ() == MMapType::Private {
        // map private
        VmoChildOptions::new_cow(page_cache_vmo, offset..(offset + len)).alloc()?
    } else {
        // map shared
        // FIXME: enable slice child to exceed parent range
        VmoChildOptions::new_slice(page_cache_vmo, offset..(offset + len)).alloc()?
    };

    let root_vmar = current.root_vmar();
    let vm_map_options = {
        let mut options = root_vmar.new_map(vmo.to_dyn(), perms)?;
        if option.flags().contains(MMapFlags::MAP_FIXED) {
            options = options.offset(addr).can_overwrite(true);
        }
        options
    };
    let map_addr = vm_map_options.build()?;
    trace!("map range = 0x{:x} - 0x{:x}", map_addr, map_addr + len);
    Ok(map_addr)
}

// Definition of MMap flags, conforming to the linux mmap interface:
// https://man7.org/linux/man-pages/man2/mmap.2.html
//
// The first 4 bits of the flag value represents the type of memory map,
// while other bits are used as memory map flags.

// The map type mask
const MAP_TYPE: u32 = 0xf;

#[derive(Copy, Clone, PartialEq, Debug, TryFromInt)]
#[repr(u8)]
pub enum MMapType {
    File = 0x0, // Invalid
    Shared = 0x1,
    Private = 0x2,
    SharedValidate = 0x3,
}

bitflags! {
    pub struct MMapFlags : u32 {
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

#[derive(Debug)]
pub struct MMapOptions {
    typ: MMapType,
    flags: MMapFlags,
}

impl TryFrom<u32> for MMapOptions {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        let typ_raw = (value & MAP_TYPE) as u8;
        let typ = MMapType::try_from(typ_raw)?;

        let flags_raw = value & !MAP_TYPE;
        let Some(flags) = MMapFlags::from_bits(flags_raw) else {
            return Err(Error::with_message(Errno::EINVAL, "unknown mmap flags"));
        };
        Ok(MMapOptions { typ, flags })
    }
}

impl MMapOptions {
    pub fn typ(&self) -> MMapType {
        self.typ
    }

    pub fn flags(&self) -> MMapFlags {
        self.flags
    }
}
