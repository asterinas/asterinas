//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use crate::{
    memory::vm_page::{VmPage, VmPageRange},
    prelude::*,
};
use core::{cmp::Ordering, ops::Range};
use kxos_frame::vm::{VmAllocOptions, VmFrameVec, VmIo, VmPerm, VmSpace};
use xmas_elf::{
    header,
    program::{self, ProgramHeader, ProgramHeader64, SegmentData},
    ElfFile,
};

use super::init_stack::InitStack;

pub struct ElfLoadInfo<'a> {
    segments: Vec<ElfSegment<'a>>,
    init_stack: InitStack,
    elf_header_info: ElfHeaderInfo,
}

pub struct ElfSegment<'a> {
    range: Range<Vaddr>,
    data: &'a [u8],
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
    pub ph_ent: usize,
}

impl<'a> ElfSegment<'a> {
    fn parse_elf_segment(segment: ProgramHeader<'a>, elf_file: &ElfFile<'a>) -> Result<Self> {
        let start = segment.virtual_addr() as Vaddr;
        let end = start + segment.mem_size() as Vaddr;
        let type_ = match segment.get_type() {
            Err(error_msg) => return_errno_with_message!(Errno::ENOEXEC, error_msg),
            Ok(type_) => type_,
        };
        let data = read_segment_data(segment, elf_file)?;
        let vm_perm = Self::parse_segment_perm(segment)?;
        Ok(Self {
            range: start..end,
            type_,
            data,
            vm_perm,
        })
    }

    pub fn parse_segment_perm(segment: ProgramHeader<'a>) -> Result<VmPerm> {
        let flags = segment.flags();
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

    pub fn is_loadable(&self) -> bool {
        self.type_ == program::Type::Load
    }

    pub fn start_address(&self) -> Vaddr {
        self.range.start
    }

    pub fn end_address(&self) -> Vaddr {
        self.range.end
    }

    fn copy_and_map_segment(&self, vm_space: &VmSpace) -> Result<()> {
        let start_address = self.start_address();
        let page_mask = PAGE_SIZE - 1;
        let segment_len = self.end_address() - self.start_address();
        let data_len = self.data.len();
        let zeroed_bytes = if segment_len > data_len {
            vec![0u8; segment_len - data_len]
        } else {
            Vec::new()
        };
        // according to linux abi, the first page may be on same page with another segment.
        // So at first, we will check whether the first page is mapped.
        if vm_space.is_mapped(start_address) {
            // The first page is mapped. This is the rare case.
            let write_len_on_first_page =
                (PAGE_SIZE - (start_address & page_mask)).min(self.data.len());
            vm_space
                .write_bytes(start_address, &self.data[..write_len_on_first_page])
                .expect("Write first page failed");
            let start_page = VmPage::containing_address(start_address).next_page();
            let end_page = VmPage::containing_address(self.end_address());
            if end_page >= start_page {
                let vm_page_range = VmPageRange::new_page_range(start_page, end_page);
                let page_num = vm_page_range.len();
                let vm_alloc_options = VmAllocOptions::new(page_num);
                let frames = VmFrameVec::allocate(&vm_alloc_options)?;
                frames.write_bytes(0, &self.data[write_len_on_first_page..])?;
                if zeroed_bytes.len() > 0 {
                    frames.write_bytes(data_len - write_len_on_first_page, &zeroed_bytes)?;
                }
                vm_page_range.map_to(vm_space, frames, self.vm_perm);
            } else {
                if zeroed_bytes.len() > 0 {
                    vm_space.write_bytes(start_address + data_len, &zeroed_bytes)?;
                }
            }
        } else {
            // The first page is not mapped. This is the common case.
            let vm_page_range = VmPageRange::new_range(start_address..self.end_address());
            let page_num = vm_page_range.len();
            let vm_alloc_options = VmAllocOptions::new(page_num);
            let frames = VmFrameVec::allocate(&vm_alloc_options)?;
            let offset = start_address & page_mask;
            // copy segment
            frames.write_bytes(offset, &self.data)?;
            // write zero bytes
            if zeroed_bytes.len() > 0 {
                let write_addr = offset + data_len;
                frames.write_bytes(write_addr, &zeroed_bytes)?;
            }
            vm_page_range.map_to(vm_space, frames, self.vm_perm);
        }
        Ok(())
    }

    fn is_page_aligned(&self) -> bool {
        self.start_address() % PAGE_SIZE == 0
    }
}

impl<'a> ElfLoadInfo<'a> {
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

    fn add_segment(&mut self, elf_segment: ElfSegment<'a>) {
        self.segments.push(elf_segment);
    }

    pub fn parse_elf_data(elf_file_content: &'a [u8], filename: CString) -> Result<Self> {
        let elf_file = match ElfFile::new(elf_file_content) {
            Err(error_msg) => return_errno_with_message!(Errno::ENOEXEC, error_msg),
            Ok(elf_file) => elf_file,
        };
        check_elf_header(&elf_file)?;
        // parse elf header
        let elf_header_info = ElfHeaderInfo::parse_elf_header(&elf_file);
        // FIXME: only contains load segment?
        let segments_count = elf_file.program_iter().count();
        let init_stack = InitStack::new_default_config(filename);
        let mut elf_load_info =
            ElfLoadInfo::with_capacity(segments_count, init_stack, elf_header_info);

        // parse each segemnt
        for segment in elf_file.program_iter() {
            let elf_segment = ElfSegment::parse_elf_segment(segment, &elf_file)?;
            if elf_segment.is_loadable() {
                elf_load_info.add_segment(elf_segment)
            }
        }

        Ok(elf_load_info)
    }

    fn vm_page_range(&self) -> Result<VmPageRange> {
        let elf_start_address = self
            .segments
            .iter()
            .filter(|segment| segment.is_loadable())
            .map(|segment| segment.start_address())
            .min()
            .unwrap();
        let elf_end_address = self
            .segments
            .iter()
            .filter(|segment| segment.is_loadable())
            .map(|segment| segment.end_address())
            .max()
            .unwrap();

        Ok(VmPageRange::new_range(elf_start_address..elf_end_address))
    }

    /// copy and map all segment
    pub fn copy_and_map_segments(&self, vm_space: &VmSpace) -> Result<()> {
        for segment in self.segments.iter() {
            segment.copy_and_map_segment(vm_space)?;
        }
        Ok(())
    }

    pub fn init_stack(&mut self, vm_space: &VmSpace) {
        self.init_stack
            .init(vm_space, &self.elf_header_info)
            .expect("Init User Stack failed");
    }

    /// This function will write the program header table to the initial stack top.
    /// This function must be called after init process initial stack.
    /// This infomation is used to set Auxv vectors.
    pub fn write_program_header_table(&self, vm_space: &VmSpace, file_content: &[u8]) {
        let write_len = PAGE_SIZE.min(file_content.len());
        let write_content = &file_content[..write_len];
        let write_addr = self.init_stack.init_stack_top() - PAGE_SIZE;
        vm_space
            .write_bytes(write_addr, write_content)
            .expect("Write elf content failed");
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

    /// read content from vmspace to ensure elf data is correctly copied to user space
    pub fn debug_check_map_result(&self, vm_space: &VmSpace) {
        for segment in self.segments.iter() {
            let start_address = segment.start_address();
            let len = segment.data.len();
            let mut read_buffer = vec![0; len];
            vm_space
                .read_bytes(start_address, &mut read_buffer)
                .expect("read bytes failed");
            let res = segment.data.cmp(&read_buffer);

            assert_eq!(res, Ordering::Equal);
        }
    }
}

impl ElfHeaderInfo {
    fn parse_elf_header(elf_file: &ElfFile) -> Self {
        let entry_point = elf_file.header.pt2.entry_point() as Vaddr;
        let ph_off = elf_file.header.pt2.ph_offset();
        let ph_num = elf_file.header.pt2.ph_count();
        let ph_ent = core::mem::size_of::<ProgramHeader64>();
        ElfHeaderInfo {
            entry_point,
            ph_off,
            ph_num,
            ph_ent,
        }
    }
}

fn check_elf_header(elf_file: &ElfFile) -> Result<()> {
    let elf_header = elf_file.header;
    // 64bit
    debug_assert_eq!(elf_header.pt1.class(), header::Class::SixtyFour);
    if elf_header.pt1.class() != header::Class::SixtyFour {
        return_errno!(Errno::ENOEXEC);
    }
    // little endian
    debug_assert_eq!(elf_header.pt1.data(), header::Data::LittleEndian);
    if elf_header.pt1.data() != header::Data::LittleEndian {
        return_errno!(Errno::ENOEXEC);
    }
    // system V ABI
    // debug_assert_eq!(elf_header.pt1.os_abi(), header::OsAbi::SystemV);
    // if elf_header.pt1.os_abi() != header::OsAbi::SystemV {
    //     return Error::new(Errno::ENOEXEC);
    // }
    // x86_64 architecture
    debug_assert_eq!(
        elf_header.pt2.machine().as_machine(),
        header::Machine::X86_64
    );
    if elf_header.pt2.machine().as_machine() != header::Machine::X86_64 {
        return_errno!(Errno::ENOEXEC);
    }
    // Executable file
    debug_assert_eq!(elf_header.pt2.type_().as_type(), header::Type::Executable);
    if elf_header.pt2.type_().as_type() != header::Type::Executable {
        return_errno!(Errno::ENOEXEC);
    }

    Ok(())
}

fn read_segment_data<'a>(segment: ProgramHeader<'a>, elf_file: &ElfFile<'a>) -> Result<&'a [u8]> {
    match segment.get_data(&elf_file) {
        Err(msg) => return_errno_with_message!(Errno::ENOEXEC, msg),
        Ok(data) => match data {
            SegmentData::Note64(_, data) | SegmentData::Undefined(data) => Ok(data),
            _ => return_errno_with_message!(Errno::ENOEXEC, "Unkonwn segment data type"),
        },
    }
}
