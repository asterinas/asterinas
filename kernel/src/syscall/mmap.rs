// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    fs::file_table::{FileDesc, get_file_fast},
    prelude::*,
    vm::{
        perms::VmPerms,
        vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR},
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
    let perms = VmPerms::from_user_bits_truncate(perms as u32);
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

    let len = check_len(len)?;
    check_addr(addr, len, option.flags())?;
    check_offset(offset, len, option.flags())?;

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
        let mut options = vmar.new_map(len, vm_perms)?;

        if option.flags().is_fixed() {
            options = options.offset(addr);
            if !option.flags().contains(MMapFlags::MAP_FIXED_NOREPLACE) {
                options = options.can_overwrite(true);
            }
        } else if option.flags().contains(MMapFlags::MAP_32BIT) {
            // TODO: Support MAP_32BIT. MAP_32BIT requires the mapping address to be below 2 GiB.
            warn!("MAP_32BIT is not supported");
        }

        if option.typ().is_shared() {
            options = options.is_shared(true);
        }

        if option.flags().contains(MMapFlags::MAP_ANONYMOUS) {
            // Anonymous shared mappings should share the same memory pages.
            if option.typ().is_shared() {
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
            if option.typ() == MMapType::Shared && !access_mode.is_writable() {
                if vm_perms.contains(VmPerms::WRITE) {
                    return_errno_with_message!(Errno::EACCES, "the file is not opened writable");
                }
                vm_may_perms.remove(VmPerms::MAY_WRITE);
            }

            options = options
                .may_perms(vm_may_perms)
                .mappable(file.as_ref().as_ref())?
                .vmo_offset(offset)
                .handle_page_faults_around();
        }

        options
    };

    let map_addr = vm_map_options.build()?;

    Ok(map_addr)
}

fn check_len(len: usize) -> Result<usize> {
    if len == 0 {
        return_errno_with_message!(Errno::EINVAL, "the mapping length is zero");
    }

    if len > VMAR_CAP_ADDR {
        return_errno_with_message!(Errno::ENOMEM, "the mapping length is too large");
    }

    Ok(len.align_up(PAGE_SIZE))
}

fn check_addr(addr: Vaddr, len: usize, flags: MMapFlags) -> Result<()> {
    if !flags.is_fixed() {
        return Ok(());
    }

    if !addr.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "the mapping address is not aligned");
    }

    if addr < VMAR_LOWEST_ADDR {
        return_errno_with_message!(Errno::EPERM, "the mapping address is too low");
    }

    if addr > VMAR_CAP_ADDR - len {
        return_errno_with_message!(Errno::ENOMEM, "the mapping address is too high");
    }

    Ok(())
}

fn check_offset(offset: usize, len: usize, flags: MMapFlags) -> Result<()> {
    if !offset.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "the mapping offset is not aligned");
    }

    if flags.contains(MMapFlags::MAP_ANONYMOUS) {
        return Ok(());
    }

    if offset
        .checked_add(len)
        .is_none_or(|end| end >= isize::MAX as usize)
    {
        return_errno_with_message!(Errno::EOVERFLOW, "the mapping offset overflows");
    }

    Ok(())
}

// Definition of mmap flags, conforming to the Linux mmap interface:
// <https://man7.org/linux/man-pages/man2/mmap.2.html>.
//
// The first 4 bits of the flag value represents the type of the mapping,
// while other bits are used as the flags of the mapping.

/// The mask for the mapping type.
const MAP_TYPE_MASK: u32 = 0xf;

#[derive(Copy, Clone, PartialEq, Debug, TryFromInt)]
#[repr(u8)]
enum MMapType {
    Shared = 0x1,
    Private = 0x2,
    SharedValidate = 0x3,
}

impl MMapType {
    pub(self) fn is_shared(self) -> bool {
        matches!(self, Self::Shared | Self::SharedValidate)
    }
}

bitflags! {
    // If you update the flags here, please also check and update `LEGACY_MMAP_FLAGS` below.
    struct MMapFlags : u32 {
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
        const MAP_FIXED_NOREPLACE = 0x100000;
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.18.1/source/include/linux/mman.h#L35-L59>
const LEGACY_MMAP_FLAGS: MMapFlags = MMapFlags::MAP_FIXED
    .union(MMapFlags::MAP_ANONYMOUS)
    .union(MMapFlags::MAP_32BIT)
    .union(MMapFlags::MAP_GROWSDOWN)
    .union(MMapFlags::MAP_DENYWRITE)
    .union(MMapFlags::MAP_EXECUTABLE)
    .union(MMapFlags::MAP_LOCKED)
    .union(MMapFlags::MAP_NORESERVE)
    .union(MMapFlags::MAP_POPULATE)
    .union(MMapFlags::MAP_NONBLOCK)
    .union(MMapFlags::MAP_STACK)
    .union(MMapFlags::MAP_HUGETLB);

impl MMapFlags {
    pub(self) fn is_fixed(self) -> bool {
        self.contains(Self::MAP_FIXED) || self.contains(Self::MAP_FIXED_NOREPLACE)
    }
}

#[derive(Debug)]
struct MMapOptions {
    typ: MMapType,
    flags: MMapFlags,
}

impl TryFrom<u32> for MMapOptions {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        let typ_raw = (value & MAP_TYPE_MASK) as u8;
        let typ = MMapType::try_from(typ_raw)?;

        // According to the Linux behavior, unknown flags are silently ignored unless
        // `MAP_SHARED_VALIDATE` is specified.
        let flags_raw = value & !MAP_TYPE_MASK;
        if typ == MMapType::SharedValidate && (flags_raw & !LEGACY_MMAP_FLAGS.bits()) != 0 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "the mapping flags are not supported");
        }
        let flags = MMapFlags::from_bits_truncate(flags_raw);

        Ok(MMapOptions { typ, flags })
    }
}

impl MMapOptions {
    pub(self) fn typ(&self) -> MMapType {
        self.typ
    }

    pub(self) fn flags(&self) -> MMapFlags {
        self.flags
    }
}
