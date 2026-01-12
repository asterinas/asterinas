// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use xmas_elf::{
    header::{self, Header, HeaderPt1, HeaderPt2, HeaderPt2_, Machine_, Type_},
    program::{self, ProgramHeader64},
};

use crate::{
    fs::utils::{Inode, PATH_MAX},
    prelude::*,
    vm::{perms::VmPerms, vmar::VMAR_CAP_ADDR},
};

/// A wrapper for the [`xmas_elf`] ELF parser.
pub struct ElfHeaders {
    elf_header: ElfHeader,
    loadable_phdrs: Vec<LoadablePhdr>,
    max_load_align: usize,
    interp_phdr: Option<InterpPhdr>,
}

impl ElfHeaders {
    /// The minimized length of a valid ELF header.
    ///
    /// [`Self::parse`] will fail with [`ENOEXEC`] if the slice is shorter than this constant. This
    /// can also be checked manually if a different error code is expected.
    ///
    /// [`ENOEXEC`]: Errno::ENOEXEC
    pub(super) const LEN: usize = size_of::<HeaderPt1>() + size_of::<HeaderPt2_64>();

    pub fn parse(input: &[u8]) -> Result<Self> {
        // Parse the ELF header.
        let header = xmas_elf::header::parse_header(input)
            .map_err(|_| Error::with_message(Errno::ENOEXEC, "the ELF header is invalid"))?;
        let elf_header = ElfHeader::parse(header)?;
        check_elf_header(&elf_header)?;

        // Validate the placement of the ELF program headers.
        let ph_count = elf_header.pt2.ph_count;
        let ph_entry_size = elf_header.pt2.ph_entry_size;
        let ph_offset = elf_header.pt2.ph_offset;
        if ph_entry_size as usize != size_of::<ProgramHeader64>() {
            return_errno_with_message!(
                Errno::ENOEXEC,
                "the size of ELF program headers is invalid"
            );
        }
        if ph_offset >= input.len() as u64
            || (input.len() as u64 - ph_offset) / (ph_entry_size as u64) < (ph_count as u64)
        {
            // TODO: Currently, the program headers must follow the ELF header immediately (they
            // must reside in `input`, the first page of the ELF file). This requirement may be
            // relaxed in the future.
            return_errno_with_message!(
                Errno::ENOEXEC,
                "the placement of ELF program headers is not supported"
            );
        }

        // Parse the ELF program headers.
        let mut loadable_phdrs = Vec::with_capacity(ph_count as usize);
        let mut max_load_align = PAGE_SIZE;
        let mut interp_phdr = None;
        for index in 0..ph_count {
            let program_header = xmas_elf::program::parse_program_header(input, header, index)
                .map_err(|_| {
                    Error::with_message(Errno::ENOEXEC, "the ELF program header is invalid")
                })?;
            let ph64 = match program_header {
                xmas_elf::program::ProgramHeader::Ph64(ph64) => *ph64,
                xmas_elf::program::ProgramHeader::Ph32(_) => {
                    return_errno_with_message!(
                        Errno::ENOEXEC,
                        "the ELF program header is not 64-bit"
                    )
                }
            };
            match ph64.get_type() {
                Ok(program::Type::Load) => {
                    loadable_phdrs.push(LoadablePhdr::parse(&ph64)?);
                    // Like Linux, we ignore any invalid alignment requirements that are not a
                    // power of two.
                    if ph64.align.is_power_of_two() {
                        max_load_align = max_load_align.max(ph64.align as usize);
                    }
                }
                Ok(program::Type::Interp) if interp_phdr.is_none() => {
                    // Like Linux, we only handle the first interpreter program header.
                    interp_phdr = Some(InterpPhdr::parse(&ph64)?);
                }
                _ => (),
            }
        }
        if loadable_phdrs.is_empty() {
            return_errno_with_message!(Errno::ENOEXEC, "there are no loadable ELF program headers");
        }

        Ok(Self {
            elf_header,
            loadable_phdrs,
            max_load_align,
            interp_phdr,
        })
    }

    /// Returns whether the ELF is a shared object.
    pub(super) fn is_shared_object(&self) -> bool {
        self.elf_header.pt2.type_.as_type() == header::Type::SharedObject
    }

    /// Returns the address of the entry point.
    pub(super) fn entry_point(&self) -> Vaddr {
        self.elf_header.pt2.entry_point as Vaddr
    }

    /// Returns the number of the program headers.
    pub(super) fn ph_count(&self) -> u16 {
        self.elf_header.pt2.ph_count
    }

    /// Returns the size of a program header.
    pub(super) fn ph_ent(&self) -> u16 {
        self.elf_header.pt2.ph_entry_size
    }

    /// Returns a reference to the loadable program headers.
    ///
    /// It is guaranteed that there is at least one loadable program header.
    pub(super) fn loadable_phdrs(&self) -> &[LoadablePhdr] {
        self.loadable_phdrs.as_slice()
    }

    /// Returns the maximum alignment of the loadable program headers.
    ///
    /// It is guaranteed that the alignment is a power of two and is at least [`PAGE_SIZE`].
    pub(super) fn max_load_align(&self) -> usize {
        self.max_load_align
    }

    /// Returns a reference to the interpreter program header.
    pub(super) fn interp_phdr(&self) -> Option<&InterpPhdr> {
        self.interp_phdr.as_ref()
    }

    /// Finds the virtual address of the program headers.
    pub(super) fn find_vaddr_of_phdrs(&self) -> Result<Vaddr> {
        let ph_offset = self.elf_header.pt2.ph_offset as usize;
        for loadable_phdrs in self.loadable_phdrs.iter() {
            if loadable_phdrs.file_range().contains(&ph_offset) {
                return Ok(loadable_phdrs.virt_range().start
                    + (ph_offset - loadable_phdrs.file_range().start));
            }
        }
        return_errno_with_message!(
            Errno::ENOEXEC,
            "the ELF program headers are not located in any segments"
        );
    }

    /// Calculates the virtual address bounds of all segments as a range.
    pub(super) fn calc_total_vaddr_bounds(&self) -> Range<Vaddr> {
        self.loadable_phdrs
            .iter()
            .map(LoadablePhdr::virt_range)
            .cloned()
            .reduce(|r1, r2| r1.start.min(r2.start)..r1.end.max(r2.end))
            .unwrap()
    }

    /// Finds the last loadable segment and returns its virtual address bounds.
    pub(super) fn find_last_vaddr_bound(&self) -> Option<Range<Vaddr>> {
        self.loadable_phdrs
            .iter()
            .max_by_key(|phdr| phdr.virt_range().end)
            .map(|phdr| phdr.virt_range().clone())
    }
}

struct ElfHeader {
    pt1: HeaderPt1,
    pt2: HeaderPt2_64,
}

impl ElfHeader {
    pub(self) fn parse(header: Header) -> Result<Self> {
        let pt1 = *header.pt1;
        let pt2 = match header.pt2 {
            HeaderPt2::Header64(header_pt2) => {
                let HeaderPt2_ {
                    type_,
                    machine,
                    version,
                    entry_point,
                    ph_offset,
                    sh_offset,
                    flags,
                    header_size,
                    ph_entry_size,
                    ph_count,
                    sh_entry_size,
                    sh_count,
                    sh_str_index,
                } = header_pt2;
                HeaderPt2_64 {
                    type_: *type_,
                    machine: *machine,
                    version: *version,
                    entry_point: *entry_point,
                    ph_offset: *ph_offset,
                    sh_offset: *sh_offset,
                    flags: *flags,
                    header_size: *header_size,
                    ph_entry_size: *ph_entry_size,
                    ph_count: *ph_count,
                    sh_entry_size: *sh_entry_size,
                    sh_count: *sh_count,
                    sh_str_index: *sh_str_index,
                }
            }
            _ => return_errno_with_message!(Errno::ENOEXEC, "the ELF file is not 64-bit"),
        };
        Ok(ElfHeader { pt1, pt2 })
    }
}

struct HeaderPt2_64 {
    type_: Type_,
    machine: Machine_,
    #[expect(dead_code)]
    version: u32,
    entry_point: u64,
    ph_offset: u64,
    #[expect(dead_code)]
    sh_offset: u64,
    #[expect(dead_code)]
    flags: u32,
    #[expect(dead_code)]
    header_size: u16,
    ph_entry_size: u16,
    ph_count: u16,
    #[expect(dead_code)]
    sh_entry_size: u16,
    #[expect(dead_code)]
    sh_count: u16,
    #[expect(dead_code)]
    sh_str_index: u16,
}

fn check_elf_header(elf_header: &ElfHeader) -> Result<()> {
    #[cfg(target_arch = "x86_64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::X86_64;
    #[cfg(target_arch = "riscv64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::RISC_V;
    // Reference: <https://loongson.github.io/LoongArch-Documentation/LoongArch-ELF-ABI-EN.html#_e_machine_identifies_the_machine>
    #[cfg(target_arch = "loongarch64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::Other(258);

    if elf_header.pt1.class() != header::Class::SixtyFour {
        return_errno_with_message!(Errno::ENOEXEC, "the ELF file is not 64-bit");
    }

    if elf_header.pt1.data() != header::Data::LittleEndian {
        return_errno_with_message!(Errno::ENOEXEC, "the ELF file is not in little endian");
    }

    // TODO: Should we check `pt1.os_abi()` or `pt1.version()`?

    if elf_header.pt2.machine.as_machine() != EXPECTED_ELF_MACHINE {
        return_errno_with_message!(
            Errno::ENOEXEC,
            "the ELF file is of a different architecture"
        );
    }

    let elf_type = elf_header.pt2.type_.as_type();
    if elf_type != header::Type::Executable && elf_type != header::Type::SharedObject {
        return_errno_with_message!(Errno::ENOEXEC, "the ELF file is not an executable");
    }

    Ok(())
}

/// A ELF program header of the type [`program::Type::Load`].
pub(super) struct LoadablePhdr {
    virt_range: Range<Vaddr>,
    file_range: Range<usize>,
    vm_perms: VmPerms,
}

impl LoadablePhdr {
    pub(self) fn parse(phdr: &ProgramHeader64) -> Result<Self> {
        debug_assert_eq!(phdr.get_type(), Ok(program::Type::Load));

        let virt_start = phdr.virtual_addr;
        let virt_end = if let Some(virt_end) = virt_start.checked_add(phdr.mem_size)
            && virt_end <= VMAR_CAP_ADDR as u64
        {
            virt_end
        } else {
            return_errno_with_message!(Errno::ENOMEM, "the mapping address is too large");
        };

        let file_start = phdr.offset;
        let Some(file_end) = file_start.checked_add(phdr.file_size) else {
            return_errno_with_message!(Errno::EINVAL, "the mapping offset overflows");
        };
        if file_end >= isize::MAX as u64 {
            return_errno_with_message!(Errno::EOVERFLOW, "the mapping offset overflows");
        }

        if phdr.mem_size == 0 {
            return_errno_with_message!(Errno::EINVAL, "the mapping length is zero");
        }
        if phdr.mem_size < phdr.file_size {
            return_errno_with_message!(
                Errno::EINVAL,
                "the mapping length is smaller than the file length"
            );
        }
        if virt_start % (PAGE_SIZE as u64) != file_start % (PAGE_SIZE as u64) {
            return_errno_with_message!(Errno::EINVAL, "the mapping address is not aligned");
        }

        Ok(Self {
            virt_range: (virt_start as Vaddr)..(virt_end as Vaddr),
            file_range: (file_start as usize)..(file_end as usize),
            vm_perms: parse_segment_perm(phdr.flags),
        })
    }

    /// Returns the virtual address range.
    ///
    /// The range is guaranteed to be below [`VMAR_CAP_ADDR`] and non-empty.
    pub(super) fn virt_range(&self) -> &Range<Vaddr> {
        &self.virt_range
    }

    /// Returns the file offset range.
    ///
    /// The range is guaranteed to be below [`i64::MAX`]. It will also be shorter than the virtual
    /// address range ([`Self::virt_range`]) and have the same offset within a page as that range.
    pub(super) fn file_range(&self) -> &Range<usize> {
        &self.file_range
    }

    /// Returns the permission to map the virtual memory.
    pub(super) fn vm_perms(&self) -> VmPerms {
        self.vm_perms
    }
}

fn parse_segment_perm(flags: xmas_elf::program::Flags) -> VmPerms {
    let mut vm_perm = VmPerms::empty();
    if flags.is_read() {
        vm_perm |= VmPerms::READ;
    }
    if flags.is_write() {
        vm_perm |= VmPerms::WRITE;
    }
    if flags.is_execute() {
        vm_perm |= VmPerms::EXEC;
    }
    vm_perm
}

/// A ELF program header of the type [`program::Type::Interp`].
pub(super) struct InterpPhdr {
    file_offset: usize,
    file_size: u16,
}

impl InterpPhdr {
    pub(self) fn parse(phdr: &ProgramHeader64) -> Result<Self> {
        debug_assert_eq!(phdr.get_type(), Ok(program::Type::Interp));

        if phdr
            .offset
            .checked_add(phdr.file_size)
            .is_none_or(|file_end| file_end > isize::MAX as u64)
        {
            return_errno_with_message!(Errno::EINVAL, "the interpreter offset overflows");
        }

        if phdr.file_size >= PATH_MAX as u64 {
            return_errno_with_message!(Errno::ENOEXEC, "the interpreter path is too long");
        }

        const { assert!(PATH_MAX as u64 <= u16::MAX as u64) };

        Ok(Self {
            file_offset: phdr.offset as usize,
            file_size: phdr.file_size as u16,
        })
    }

    /// Reads the LDSO path from the ELF inode.
    pub(super) fn read_ldso_path(&self, elf_inode: &Arc<dyn Inode>) -> Result<CString> {
        // Note that `self.file_size` is at most `PATH_SIZE`.
        let file_size = self.file_size as usize;
        let mut buffer = vec![0; file_size];
        if elf_inode.read_bytes_at(self.file_offset, &mut buffer)? != file_size {
            return_errno_with_message!(Errno::EIO, "the interpreter path cannot be fully read");
        }

        let ldso_path = CString::from_vec_with_nul(buffer).map_err(|_| {
            Error::with_message(
                Errno::ENOEXEC,
                "the interpreter path is not a valid C string",
            )
        })?;
        Ok(ldso_path)
    }
}
