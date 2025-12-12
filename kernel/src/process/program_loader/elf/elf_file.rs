// SPDX-License-Identifier: MPL-2.0

use xmas_elf::{
    header::{self, Header, HeaderPt1, HeaderPt2, HeaderPt2_, Machine_, Type_},
    program::{self, ProgramHeader64},
};

use crate::{
    fs::utils::{Inode, PATH_MAX},
    prelude::*,
};

/// A wrapper for the [`xmas_elf`] ELF parser.
pub struct ElfHeaders {
    elf_header: ElfHeader,
    program_headers: Vec<ProgramHeader64>,
}

impl ElfHeaders {
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
        let mut program_headers = Vec::with_capacity(ph_count as usize);
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
            program_headers.push(ph64);
        }

        Ok(Self {
            elf_header,
            program_headers,
        })
    }

    /// Returns the address of the entry point.
    pub(super) fn entry_point(&self) -> Vaddr {
        self.elf_header.pt2.entry_point as Vaddr
    }

    /// Returns a reference to the program headers.
    pub(super) fn program_headers(&self) -> &[ProgramHeader64] {
        self.program_headers.as_slice()
    }

    /// Returns the number of the program headers.
    pub(super) fn ph_count(&self) -> u16 {
        self.elf_header.pt2.ph_count
    }

    /// Returns the size of a program header.
    pub(super) fn ph_ent(&self) -> u16 {
        self.elf_header.pt2.ph_entry_size
    }

    /// Finds the virtual address of the program headers.
    pub(super) fn find_vaddr_of_phdrs(&self) -> Result<Vaddr> {
        let ph_offset = self.elf_header.pt2.ph_offset;
        for program_header in &self.program_headers {
            if let Some(offset_in_ph) = ph_offset.checked_sub(program_header.offset)
                && offset_in_ph <= program_header.file_size
            {
                return Ok((offset_in_ph + program_header.virtual_addr) as Vaddr);
            }
        }
        return_errno_with_message!(
            Errno::ENOEXEC,
            "the ELF program headers are not located in any segments"
        );
    }

    /// Returns whether the ELF is a shared object.
    pub(super) fn is_shared_object(&self) -> bool {
        self.elf_header.pt2.type_.as_type() == header::Type::SharedObject
    }

    /// Reads the LDSO path from the ELF inode.
    pub(super) fn read_ldso_path(&self, elf_inode: &Arc<dyn Inode>) -> Result<Option<CString>> {
        for program_header in &self.program_headers {
            if let Ok(program::Type::Interp) = program_header.get_type() {
                let file_size = program_header.file_size as usize;
                let file_offset = program_header.offset as usize;

                if file_size > PATH_MAX {
                    return_errno_with_message!(Errno::ENOEXEC, "the interpreter path is too long");
                }

                let mut buffer = vec![0; file_size];
                elf_inode.read_bytes_at(file_offset, &mut buffer)?;

                let ldso_path = CString::from_vec_with_nul(buffer).map_err(|_| {
                    Error::with_message(
                        Errno::ENOEXEC,
                        "the interpreter path is not a valid C string",
                    )
                })?;

                return Ok(Some(ldso_path));
            }
        }

        Ok(None)
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
