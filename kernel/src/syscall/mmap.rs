// SPDX-License-Identifier: MPL-2.0

//! This mod defines mmap flags and the handler to syscall mmap

use align_ext::AlignExt;
use aster_rights::Rights;

use super::SyscallReturn;
use crate::{
    fs::{file_handle::FileLike, file_table::FileDesc, inode_handle::InodeHandle},
    prelude::*,
    vm::{
        perms::VmPerms,
        vmo::{VmoOptions, VmoRightsOp},
    },
};

pub fn sys_mmap(
    addr: u64,
    len: u64,
    perms: u64,
    flags: u64,
    fd: u64,
    offset: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let perms = VmPerms::from_bits_truncate(perms as u32);
    let option = MMapOptions::try_from(flags as u32)?;
    let res = do_sys_mmap(
        addr as usize,
        len as usize,
        perms,
        option,
        fd as _,
        offset as usize,
        ctx,
    )?;
    Ok(SyscallReturn::Return(res as _))
}

fn do_sys_mmap(
    addr: Vaddr,
    len: usize,
    vm_perms: VmPerms,
    option: MMapOptions,
    fd: FileDesc,
    offset: usize,
    ctx: &Context,
) -> Result<Vaddr> {
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, perms = {:?}, option = {:?}, fd = {}, offset = 0x{:x}",
        addr, len, vm_perms, option, fd, offset
    );

    check_option(&option)?;

    if len == 0 {
        return_errno_with_message!(Errno::EINVAL, "mmap len cannot be zero");
    }
    if len > isize::MAX as usize {
        return_errno_with_message!(Errno::ENOMEM, "mmap len too large");
    }

    let len = len.align_up(PAGE_SIZE);

    if offset % PAGE_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "mmap only support page-aligned offset");
    }
    offset.checked_add(len).ok_or(Error::with_message(
        Errno::EOVERFLOW,
        "integer overflow when (offset + len)",
    ))?;
    if addr > isize::MAX as usize - len {
        return_errno_with_message!(Errno::ENOMEM, "mmap (addr + len) too large");
    }

    let root_vmar = ctx.process.root_vmar();
    let vm_map_options = {
        let mut options = root_vmar.new_map(len, vm_perms)?;
        let flags = option.flags;
        if flags.contains(MMapFlags::MAP_FIXED) {
            options = options.offset(addr).can_overwrite(true);
        } else if flags.contains(MMapFlags::MAP_32BIT) {
            // TODO: support MAP_32BIT. MAP_32BIT requires the map range to be below 2GB
            warn!("MAP_32BIT is not supported");
        }

        if option.typ() == MMapType::Shared {
            options = options.is_shared(true);
        }

        if option.flags.contains(MMapFlags::MAP_ANONYMOUS) {
            if offset != 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "offset must be zero for anonymous mapping"
                );
            }

            // Anonymous shared mapping should share the same memory pages.
            if option.typ() == MMapType::Shared {
                let shared_vmo = {
                    let vmo_options: VmoOptions<Rights> = VmoOptions::new(len);
                    vmo_options.alloc()?
                };
                options = options.vmo(shared_vmo);
            }
        } else {
            let vmo = {
                let file_table = ctx.process.file_table().lock();
                let file = file_table.get_file(fd)?;
                let inode_handle = file
                    .downcast_ref::<InodeHandle>()
                    .ok_or(Error::with_message(Errno::EINVAL, "no inode"))?;

                let access_mode = inode_handle.access_mode();
                if vm_perms.contains(VmPerms::READ) && !access_mode.is_readable() {
                    return_errno!(Errno::EACCES);
                }
                if option.typ() == MMapType::Shared
                    && vm_perms.contains(VmPerms::WRITE)
                    && !access_mode.is_writable()
                {
                    return_errno!(Errno::EACCES);
                }

                let inode = inode_handle.dentry().inode();
                inode
                    .page_cache()
                    .ok_or(Error::with_message(
                        Errno::EBADF,
                        "File does not have page cache",
                    ))?
                    .to_dyn()
            };

            options = options
                .vmo(vmo)
                .vmo_offset(offset)
                .handle_page_faults_around();
        }

        options
    };

    let map_addr = vm_map_options.build()?;
    Ok(map_addr)
}

fn check_option(option: &MMapOptions) -> Result<()> {
    if option.typ() == MMapType::File {
        return_errno_with_message!(Errno::EINVAL, "Invalid mmap type");
    }

    Ok(())
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
        const MAP_32BIT           = 0x40;
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
