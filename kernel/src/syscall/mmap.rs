// SPDX-License-Identifier: MPL-2.0

//! This mod defines mmap flags and the handler to syscall mmap

use core::num::NonZeroUsize;

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    fs::file_table::{FileDesc, get_file_fast},
    prelude::*,
    vm::{
        perms::VmPerms,
        vmar::{OffsetType, is_userspace_vaddr},
        vmo::VmoOptions,
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

    if option.typ == MMapType::File {
        return_errno_with_message!(Errno::EINVAL, "invalid mmap type");
    }

    let MMapAddrSizeOptions { addr_options, size } = get_addr_options(addr, len, option.flags)?;

    // `PROT_NONE` mappings are not populated.
    let populate =
        option.flags.contains(MMapFlags::MAP_POPULATE) && vm_perms.contains(VmPerms::READ);

    // On x86_64 and riscv64, `PROT_WRITE` implies `PROT_READ`.
    // Reference:
    // <https://man7.org/linux/man-pages/man2/mmap.2.html>,
    // Section 5.11.3 from <https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-3a-part-1-manual.pdf>,
    // <https://riscv.github.io/riscv-isa-manual/snapshot/privileged/#translation>.
    #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
    let vm_perms = if !vm_perms.contains(VmPerms::READ) && vm_perms.contains(VmPerms::WRITE) {
        vm_perms | VmPerms::READ
    } else {
        vm_perms
    };

    let mut vm_may_perms = VmPerms::ALL_MAY_PERMS;

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();
    let vm_map_options = {
        let mut options = vmar.new_map(size, vm_perms)?;
        let flags = option.flags;

        if let Some((addr, typ)) = addr_options {
            options = options.offset(addr, typ);
        }

        if populate {
            options = options.populate();
        }

        if flags.contains(MMapFlags::MAP_32BIT) {
            // TODO: support MAP_32BIT. MAP_32BIT requires the map range to be below 2GB
            warn!("MAP_32BIT is not supported");
        }

        if option.typ == MMapType::Shared {
            options = options.is_shared(true);
        }

        if option.flags.contains(MMapFlags::MAP_ANONYMOUS) {
            if offset != 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the offset must be zero for anonymous mapping"
                );
            }

            // Anonymous shared mapping should share the same memory pages.
            if option.typ == MMapType::Shared {
                let shared_vmo = {
                    let vmo_options = VmoOptions::new(len);
                    vmo_options.alloc()?
                };
                options = options.vmo(shared_vmo);
            }
        } else {
            let mut file_table = ctx.thread_local.borrow_file_table_mut();
            let file = get_file_fast!(&mut file_table, fd);

            let access_mode = file.access_mode();
            if vm_perms.contains(VmPerms::READ) && !access_mode.is_readable() {
                return_errno_with_message!(Errno::EACCES, "the file is not opened readable");
            }
            if option.typ == MMapType::Shared && !access_mode.is_writable() {
                if vm_perms.contains(VmPerms::WRITE) {
                    return_errno_with_message!(Errno::EACCES, "the file is not opened writable");
                }
                vm_may_perms.remove(VmPerms::MAY_WRITE);
            }

            options = options
                .may_perms(vm_may_perms)
                .mappable(file.mappable()?)
                .vmo_offset(offset)
                .handle_page_faults_around();
        }

        options
    };

    let map_addr = vm_map_options.build()?;

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

#[derive(Debug)]
struct MMapAddrSizeOptions {
    addr_options: Option<(Vaddr, OffsetType)>,
    size: NonZeroUsize,
}

fn get_addr_options(offset: Vaddr, size: usize, flags: MMapFlags) -> Result<MMapAddrSizeOptions> {
    if size > isize::MAX as usize {
        return_errno_with_message!(Errno::ENOMEM, "mmap: size too large");
    }
    if size == 0 {
        return_errno_with_message!(Errno::EINVAL, "mmap: size is zero");
    }
    let size_aligned = NonZeroUsize::new(size.align_up(PAGE_SIZE)).unwrap();

    let end = offset
        .checked_add(size_aligned.get())
        .ok_or(Error::with_message(
            Errno::EINVAL,
            "mmap: integer overflow when calculating offset + size",
        ))?;
    if end > isize::MAX as usize {
        return_errno_with_message!(Errno::ENOMEM, "mmap: offset + size too large");
    }

    let offset_aligned = offset.align_down(PAGE_SIZE);

    let range_in_userspace = is_userspace_vaddr(offset_aligned) && is_userspace_vaddr(end - 1);

    if flags.contains(MMapFlags::MAP_FIXED) | flags.contains(MMapFlags::MAP_FIXED_NOREPLACE) {
        if !offset.is_multiple_of(PAGE_SIZE) {
            return_errno_with_message!(Errno::EINVAL, "mmap: offset not page-aligned");
        }
        if !range_in_userspace {
            return_errno_with_message!(Errno::EINVAL, "mmap: fixed address not in userspace");
        }

        let typ = if flags.contains(MMapFlags::MAP_FIXED_NOREPLACE) {
            OffsetType::FixedNoReplace
        } else {
            OffsetType::Fixed
        };

        Ok(MMapAddrSizeOptions {
            addr_options: Some((offset_aligned, typ)),
            size: size_aligned,
        })
    } else if range_in_userspace {
        Ok(MMapAddrSizeOptions {
            addr_options: Some((offset_aligned, OffsetType::Hint)),
            size: size_aligned,
        })
    } else {
        Ok(MMapAddrSizeOptions {
            addr_options: None,
            size: size_aligned,
        })
    }
}
