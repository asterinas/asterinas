// SPDX-License-Identifier: MPL-2.0

/// A wrapper of xmas_elf's elf parsing
use xmas_elf::{
    header::{self, Header, HeaderPt1, HeaderPt2, HeaderPt2_, Machine_, Type_},
    program::{self, ProgramHeader64},
};

use crate::prelude::*;
pub struct Elf {
    pub elf_header: ElfHeader,
    pub program_headers: Vec<ProgramHeader64>,
}

impl Elf {
    pub fn parse_elf(input: &[u8]) -> Result<Self> {
        // first parse elf header
        // The elf header is usually 64 bytes. pt1 is 16bytes and pt2 is 48 bytes.
        // We require 128 bytes here is to keep consistency with linux implementations.
        debug_assert!(input.len() >= 128);
        let header = xmas_elf::header::parse_header(input)
            .map_err(|_| Error::with_message(Errno::ENOEXEC, "parse elf header fails"))?;
        let elf_header = ElfHeader::parse_elf_header(header)?;
        check_elf_header(&elf_header)?;
        // than parse the program headers table
        // FIXME: we should acquire enough pages before parse
        let ph_offset = elf_header.pt2.ph_offset;
        let ph_count = elf_header.pt2.ph_count;
        let ph_entry_size = elf_header.pt2.ph_entry_size;
        debug_assert!(
            input.len() >= ph_offset as usize + ph_count as usize * ph_entry_size as usize
        );
        let mut program_headers = Vec::with_capacity(ph_count as usize);
        for index in 0..ph_count {
            let program_header = xmas_elf::program::parse_program_header(input, header, index)
                .map_err(|_| Error::with_message(Errno::ENOEXEC, "parse program header fails"))?;
            let ph64 = match program_header {
                xmas_elf::program::ProgramHeader::Ph64(ph64) => *ph64,
                xmas_elf::program::ProgramHeader::Ph32(_) => {
                    return_errno_with_message!(Errno::ENOEXEC, "Not 64 byte executable")
                }
            };
            program_headers.push(ph64);
        }
        Ok(Self {
            elf_header,
            program_headers,
        })
    }

    // The following info is used to setup init stack
    /// the entry point of the elf
    pub fn entry_point(&self) -> Vaddr {
        self.elf_header.pt2.entry_point as Vaddr
    }
    /// program header table offset
    pub fn ph_off(&self) -> u64 {
        self.elf_header.pt2.ph_offset
    }
    /// number of program headers
    pub fn ph_count(&self) -> u16 {
        self.elf_header.pt2.ph_count
    }
    /// The size of a program header
    pub fn ph_ent(&self) -> u16 {
        self.elf_header.pt2.ph_entry_size
    }

    /// The virtual addr of program headers table address
    pub fn ph_addr(&self) -> Result<Vaddr> {
        let ph_offset = self.ph_off();
        for program_header in &self.program_headers {
            if program_header.offset <= ph_offset
                && ph_offset < program_header.offset + program_header.file_size
            {
                return Ok(
                    (ph_offset - program_header.offset + program_header.virtual_addr) as Vaddr,
                );
            }
        }
        return_errno_with_message!(
            Errno::ENOEXEC,
            "can not find program header table address in elf"
        );
    }

    /// whether the elf is a shared object
    pub fn is_shared_object(&self) -> bool {
        self.elf_header.pt2.type_.as_type() == header::Type::SharedObject
    }

    /// read the ldso path from the elf interpret section
    pub fn ldso_path(&self, file_header_buf: &[u8]) -> Result<Option<String>> {
        for program_header in &self.program_headers {
            let type_ = program_header.get_type().map_err(|_| {
                Error::with_message(Errno::ENOEXEC, "parse program header type fails")
            })?;
            if type_ == program::Type::Interp {
                let file_size = program_header.file_size as usize;
                let file_offset = program_header.offset as usize;
                debug_assert!(file_offset + file_size <= file_header_buf.len());
                let ldso = CStr::from_bytes_with_nul(
                    &file_header_buf[file_offset..file_offset + file_size],
                )?;
                return Ok(Some(ldso.to_string_lossy().to_string()));
            }
        }
        Ok(None)
    }

    // An offset to be subtracted from ELF vaddr for PIE
    pub fn base_load_address_offset(&self) -> u64 {
        let phdr = self.program_headers.first().unwrap();
        phdr.virtual_addr - phdr.offset
    }
}

pub struct ElfHeader {
    pub pt1: HeaderPt1,
    pub pt2: HeaderPt2_64,
}

impl ElfHeader {
    fn parse_elf_header(header: Header) -> Result<Self> {
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
            _ => return_errno_with_message!(Errno::ENOEXEC, "parse elf header failed"),
        };
        Ok(ElfHeader { pt1, pt2 })
    }
}

pub struct HeaderPt2_64 {
    pub type_: Type_,
    pub machine: Machine_,
    #[allow(dead_code)]
    pub version: u32,
    pub entry_point: u64,
    pub ph_offset: u64,
    #[allow(dead_code)]
    pub sh_offset: u64,
    #[allow(dead_code)]
    pub flags: u32,
    #[allow(dead_code)]
    pub header_size: u16,
    pub ph_entry_size: u16,
    pub ph_count: u16,
    #[allow(dead_code)]
    pub sh_entry_size: u16,
    #[allow(dead_code)]
    pub sh_count: u16,
    #[allow(dead_code)]
    pub sh_str_index: u16,
}

fn check_elf_header(elf_header: &ElfHeader) -> Result<()> {
    #[cfg(target_arch = "riscv64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::RISC_V;
    #[cfg(target_arch = "x86_64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::X86_64;

    // 64bit
    debug_assert_eq!(elf_header.pt1.class(), header::Class::SixtyFour);
    if elf_header.pt1.class() != header::Class::SixtyFour {
        return_errno_with_message!(Errno::ENOEXEC, "Not 64 byte executable");
    }
    // little endian
    debug_assert_eq!(elf_header.pt1.data(), header::Data::LittleEndian);
    if elf_header.pt1.data() != header::Data::LittleEndian {
        return_errno_with_message!(Errno::ENOEXEC, "Not little endian executable");
    }
    // system V ABI
    // debug_assert_eq!(elf_header.pt1.os_abi(), header::OsAbi::SystemV);
    // if elf_header.pt1.os_abi() != header::OsAbi::SystemV {
    //     return Error::new(Errno::ENOEXEC);
    // }
    if elf_header.pt2.machine.as_machine() != EXPECTED_ELF_MACHINE {
        return_errno_with_message!(
            Errno::ENOEXEC,
            "Executable could not be run on this architecture"
        );
    }
    // Executable file or shared object
    let elf_type = elf_header.pt2.type_.as_type();
    if elf_type != header::Type::Executable && elf_type != header::Type::SharedObject {
        return_errno_with_message!(Errno::ENOEXEC, "Not executable file");
    }

    Ok(())
}
