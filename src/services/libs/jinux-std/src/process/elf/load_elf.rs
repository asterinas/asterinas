//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use crate::vm::perms::VmPerms;
use crate::vm::vmo::VmoRightsOp;
use crate::{
    prelude::*,
    rights::Full,
    vm::{
        vmar::Vmar,
        vmo::{Pager, Vmo, VmoOptions},
    },
};
use jinux_frame::vm::VmPerm;
use jinux_frame::AlignExt;
use xmas_elf::program::{self, ProgramHeader64};

use super::elf_file::Elf;
use super::elf_segment_pager::ElfSegmentPager;
use super::init_stack::InitStack;

pub struct ElfLoadInfo {
    segments: Vec<ElfSegment>,
    init_stack: InitStack,
    elf_header_info: ElfHeaderInfo,
}

pub struct ElfSegment {
    /// The virtual addr where to put the segment.
    pub virtual_addr: Vaddr,
    /// The segment's size in memory, in bytes.
    pub mem_size: usize,
    /// The segment's offset in origin elf file
    pub offset: usize,
    /// The size the segment has in origin elf file, in bytes
    pub file_size: usize,
    type_: program::Type,
    vm_perm: VmPerm,
}

#[derive(Debug, Clone, Copy, Default)]
/// Info parsed from elf header. The entry point is used to set rip
/// The other info is used to set auxv vectors.
pub struct ElfHeaderInfo {
    /// the entry point of the elf
    pub entry_point: Vaddr,
    /// page header table offset
    pub ph_off: u64,
    /// number of program headers
    pub ph_num: u16,
    /// The size of a program header
    pub ph_ent: u16,
}

impl ElfSegment {
    fn parse_elf_segment(program_header: ProgramHeader64) -> Result<Self> {
        let start = program_header.virtual_addr as Vaddr;
        let end = start + program_header.mem_size as Vaddr;
        let type_ = program_header
            .get_type()
            .map_err(|_| Error::new(Errno::ENOEXEC))?;
        let vm_perm = Self::parse_segment_perm(program_header.flags)?;
        Ok(Self {
            virtual_addr: program_header.virtual_addr as _,
            mem_size: program_header.mem_size as usize,
            offset: program_header.offset as usize,
            file_size: program_header.file_size as usize,
            type_,
            vm_perm,
        })
    }

    pub fn parse_segment_perm(flags: xmas_elf::program::Flags) -> Result<VmPerm> {
        if !flags.is_read() {
            return_errno_with_message!(Errno::ENOEXEC, "unreadable segment");
        }
        let mut vm_perm = VmPerm::R;
        if flags.is_write() {
            vm_perm |= VmPerm::W;
        }
        if flags.is_execute() {
            vm_perm |= VmPerm::X;
        }
        Ok(vm_perm)
    }

    fn contains_program_headers_table(&self, ph_offset: usize) -> bool {
        // program headers table is at ph_offset of elf file
        self.offset <= ph_offset && ph_offset < self.offset + self.file_size
    }

    /// If this segment contains ph table, then returns the ph table addr
    /// Otherwise, returns None
    pub fn program_headers_table_addr(&self, ph_offset: usize) -> Option<Vaddr> {
        if self.contains_program_headers_table(ph_offset) {
            Some(ph_offset - self.offset + self.virtual_addr)
        } else {
            None
        }
    }

    pub fn is_loadable(&self) -> bool {
        self.type_ == program::Type::Load
    }

    pub fn start_address(&self) -> Vaddr {
        self.virtual_addr
    }

    pub fn end_address(&self) -> Vaddr {
        self.virtual_addr + self.mem_size
    }

    pub fn init_segment_vmo(&self, elf_file_content: &'static [u8]) -> Vmo<Full> {
        let vmo_start = self.start_address().align_down(PAGE_SIZE);
        let vmo_end = self.end_address().align_up(PAGE_SIZE);
        let segment_len = vmo_end - vmo_start;
        let pager = Arc::new(ElfSegmentPager::new(elf_file_content, self)) as Arc<dyn Pager>;
        let vmo_alloc_options: VmoOptions<Full> = VmoOptions::new(segment_len).pager(pager);
        vmo_alloc_options.alloc().unwrap()
    }

    // create vmo for each segment and map the segment to root_vmar
    fn map_segment_vmo(
        &self,
        root_vmar: &Vmar<Full>,
        elf_file_content: &'static [u8],
    ) -> Result<()> {
        let vmo = self.init_segment_vmo(elf_file_content).to_dyn();
        let perms = VmPerms::from(self.vm_perm);
        // The segment may not be aligned to page
        let offset = self.start_address().align_down(PAGE_SIZE);
        let vm_map_options = root_vmar.new_map(vmo, perms)?.offset(offset);
        let map_addr = vm_map_options.build()?;
        Ok(())
    }
}

impl ElfLoadInfo {
    fn with_capacity(
        capacity: usize,
        init_stack: InitStack,
        elf_header_info: ElfHeaderInfo,
    ) -> Self {
        Self {
            segments: Vec::with_capacity(capacity),
            init_stack,
            elf_header_info,
        }
    }

    fn add_segment(&mut self, elf_segment: ElfSegment) {
        self.segments.push(elf_segment);
    }

    pub fn parse_elf_data(
        elf_file_content: &'static [u8],
        argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Self> {
        let elf_file = Elf::parse_elf(elf_file_content)?;
        // parse elf header
        let elf_header_info = ElfHeaderInfo::parse_elf_header(&elf_file);
        // FIXME: only contains load segment?
        let ph_count = elf_file.program_headers.len();
        let init_stack = InitStack::new_default_config(argv, envp);
        let mut elf_load_info = ElfLoadInfo::with_capacity(ph_count, init_stack, elf_header_info);

        // parse each segemnt
        for program_header in elf_file.program_headers {
            let elf_segment = ElfSegment::parse_elf_segment(program_header)?;
            if elf_segment.is_loadable() {
                elf_load_info.add_segment(elf_segment)
            }
        }

        Ok(elf_load_info)
    }

    /// init vmo for each segment and then map segment to root vmar
    pub fn map_segment_vmos(
        &self,
        root_vmar: &Vmar<Full>,
        elf_file_content: &'static [u8],
    ) -> Result<()> {
        for segment in &self.segments {
            segment.map_segment_vmo(root_vmar, elf_file_content)?;
        }
        Ok(())
    }

    pub fn init_stack(&mut self, root_vmar: &Vmar<Full>, file_content: &[u8]) -> Result<()> {
        let ph_addr = self.program_headers_table_addr()?;
        self.init_stack
            .init(root_vmar, &self.elf_header_info, ph_addr)?;
        Ok(())
    }

    fn program_headers_table_addr(&self) -> Result<Vaddr> {
        let ph_offset = self.elf_header_info.ph_off as usize;
        for segment in &self.segments {
            if let Some(ph_addr) = segment.program_headers_table_addr(ph_offset) {
                return Ok(ph_addr);
            }
        }
        return_errno_with_message!(
            Errno::ENOEXEC,
            "can not find program header table address in elf"
        );
    }

    pub fn entry_point(&self) -> u64 {
        self.elf_header_info.entry_point as u64
    }

    pub fn user_stack_top(&self) -> u64 {
        self.init_stack.user_stack_top() as u64
    }

    pub fn argc(&self) -> u64 {
        self.init_stack.argc()
    }

    pub fn argv(&self) -> u64 {
        self.init_stack.argv()
    }

    pub fn envc(&self) -> u64 {
        self.init_stack.envc()
    }

    pub fn envp(&self) -> u64 {
        self.init_stack.envp()
    }
}

impl ElfHeaderInfo {
    fn parse_elf_header(elf_file: &Elf) -> Self {
        let entry_point = elf_file.elf_header.pt2.entry_point as Vaddr;
        let ph_off = elf_file.elf_header.pt2.ph_offset;
        let ph_num = elf_file.elf_header.pt2.ph_count;
        let ph_ent = elf_file.elf_header.pt2.ph_entry_size;
        ElfHeaderInfo {
            entry_point,
            ph_off,
            ph_num,
            ph_ent,
        }
    }
}
