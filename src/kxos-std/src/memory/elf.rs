//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use crate::prelude::*;
use core::{cmp::Ordering, ops::Range};
use kxos_frame::{
    vm::{VmIo, VmPerm, VmSpace},
    Error,
};
use xmas_elf::{
    header,
    program::{self, ProgramHeader, ProgramHeader64, SegmentData},
    ElfFile,
};

use super::{init_stack::InitStack, vm_page::VmPageRange};

pub struct ElfLoadInfo<'a> {
    entry_point: Vaddr,
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
/// Info parsed from elf header. Used to set aux vector.
pub struct ElfHeaderInfo {
    /// page header table offset
    pub ph_off: u64,
    /// number of program headers
    pub ph_num: u16,
    /// The size of a program header
    pub ph_ent: usize,
}

impl<'a> ElfSegment<'a> {
    fn parse_elf_segment(
        segment: ProgramHeader<'a>,
        elf_file: &ElfFile<'a>,
    ) -> Result<Self, ElfError> {
        let start = segment.virtual_addr() as Vaddr;
        let end = start + segment.mem_size() as Vaddr;
        let type_ = match segment.get_type() {
            Err(error_msg) => return Err(ElfError::from(error_msg)),
            Ok(type_) => type_,
        };
        let data = match read_segment_data(segment, elf_file) {
            Err(msg) => return Err(ElfError::from(msg)),
            Ok(data) => data,
        };
        let vm_perm = Self::parse_segment_perm(segment)?;
        Ok(Self {
            range: start..end,
            type_,
            data,
            vm_perm,
        })
    }

    pub fn parse_segment_perm(segment: ProgramHeader<'a>) -> Result<VmPerm, ElfError> {
        let flags = segment.flags();
        if !flags.is_read() {
            return Err(ElfError::UnreadableSegment);
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

    fn copy_segment(&self, vm_space: &VmSpace) -> Result<(), ElfError> {
        let vm_page_range = VmPageRange::new_range(self.start_address()..self.end_address());
        for page in vm_page_range.iter() {
            // map page if the page is not mapped
            if !page.is_mapped(vm_space) {
                let vm_perm = self.vm_perm | VmPerm::W;
                page.map_page(vm_space, vm_perm)?;
            }
        }
        // copy segment
        vm_space.write_bytes(self.start_address(), self.data)?;

        // The length of segment may be greater than the length of data
        // In this case, the additional bytes should be zeroed.
        let segment_len = self.end_address() - self.start_address();
        let data_len = self.data.len();
        if segment_len > data_len {
            let zeroed_bytes = vec![0u8; segment_len - data_len];
            let write_addr = self.start_address() + data_len;
            vm_space
                .write_bytes(write_addr, &zeroed_bytes)
                .expect("Write zeroed bytes failed");
        }
        Ok(())
    }

    fn is_page_aligned(&self) -> bool {
        self.start_address() % PAGE_SIZE == 0
    }
}

impl<'a> ElfLoadInfo<'a> {
    fn with_capacity(
        entry_point: Vaddr,
        capacity: usize,
        init_stack: InitStack,
        elf_header_info: ElfHeaderInfo,
    ) -> Self {
        Self {
            entry_point,
            segments: Vec::with_capacity(capacity),
            init_stack,
            elf_header_info,
        }
    }

    fn add_segment(&mut self, elf_segment: ElfSegment<'a>) {
        self.segments.push(elf_segment);
    }

    pub fn parse_elf_data(elf_file_content: &'a [u8], filename: CString) -> Result<Self, ElfError> {
        let elf_file = match ElfFile::new(elf_file_content) {
            Err(error_msg) => return Err(ElfError::from(error_msg)),
            Ok(elf_file) => elf_file,
        };
        check_elf_header(&elf_file)?;
        // init elf load info
        let entry_point = elf_file.header.pt2.entry_point() as Vaddr;
        let elf_header_info = ElfHeaderInfo::parse_elf_header(&elf_file);
        // FIXME: only contains load segment?
        let segments_count = elf_file.program_iter().count();
        let init_stack = InitStack::new_default_config(filename);
        let mut elf_load_info =
            ElfLoadInfo::with_capacity(entry_point, segments_count, init_stack, elf_header_info);

        // parse each segemnt
        for segment in elf_file.program_iter() {
            let elf_segment = ElfSegment::parse_elf_segment(segment, &elf_file)?;
            if elf_segment.is_loadable() {
                elf_load_info.add_segment(elf_segment)
            }
        }

        Ok(elf_load_info)
    }

    fn vm_page_range(&self) -> Result<VmPageRange, ElfError> {
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

    pub fn copy_data(&self, vm_space: &VmSpace) -> Result<(), ElfError> {
        for segment in self.segments.iter() {
            segment.copy_segment(vm_space)?;
        }
        Ok(())
    }

    pub fn init_stack(&mut self, vm_space: &VmSpace) {
        self.init_stack
            .init(vm_space, &self.elf_header_info)
            .expect("Init User Stack failed");
    }

    /// This function will write the first page of elf file to the initial stack top.
    /// This function must be called after init process initial stack.
    /// This infomation is used to set Auxv vectors.
    pub fn write_elf_first_page(&self, vm_space: &VmSpace, file_content: &[u8]) {
        let write_len = PAGE_SIZE.min(file_content.len());
        let write_content = &file_content[..write_len];
        let write_addr = self.init_stack.init_stack_top() - PAGE_SIZE;
        vm_space
            .write_bytes(write_addr, write_content)
            .expect("Write elf content failed");
    }

    /// return the perm of elf pages
    /// FIXME: Set the correct permission bit of user pages.
    fn perm() -> VmPerm {
        VmPerm::RXU
    }

    pub fn entry_point(&self) -> u64 {
        self.entry_point as u64
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
            // if res != Ordering::Equal {
            //     debug!("segment: 0x{:x} - 0x{:x}", segment.start_address(), segment.end_address());
            //     debug!("read buffer len: 0x{:x}", read_buffer.len());
            //     for i in 0..segment.data.len() {
            //         if segment.data[i] != read_buffer[i] {
            //             debug!("i = 0x{:x}", i);
            //             break;
            //         }
            //     }
            // }

            assert_eq!(res, Ordering::Equal);
        }
    }
}

impl ElfHeaderInfo {
    fn parse_elf_header(elf_file: &ElfFile) -> Self {
        let ph_off = elf_file.header.pt2.ph_offset();
        let ph_num = elf_file.header.pt2.ph_count();
        let ph_ent = core::mem::size_of::<ProgramHeader64>();
        ElfHeaderInfo {
            ph_off,
            ph_num,
            ph_ent,
        }
    }
}

fn check_elf_header(elf_file: &ElfFile) -> Result<(), ElfError> {
    let elf_header = elf_file.header;
    // 64bit
    debug_assert_eq!(elf_header.pt1.class(), header::Class::SixtyFour);
    if elf_header.pt1.class() != header::Class::SixtyFour {
        return Err(ElfError::UnsupportedElfType);
    }
    // little endian
    debug_assert_eq!(elf_header.pt1.data(), header::Data::LittleEndian);
    if elf_header.pt1.data() != header::Data::LittleEndian {
        return Err(ElfError::UnsupportedElfType);
    }
    // system V ABI
    // debug_assert_eq!(elf_header.pt1.os_abi(), header::OsAbi::SystemV);
    // if elf_header.pt1.os_abi() != header::OsAbi::SystemV {
    //     return Err(ElfError::UnsupportedElfType);
    // }
    // x86_64 architecture
    debug_assert_eq!(
        elf_header.pt2.machine().as_machine(),
        header::Machine::X86_64
    );
    if elf_header.pt2.machine().as_machine() != header::Machine::X86_64 {
        return Err(ElfError::UnsupportedElfType);
    }
    // Executable file
    debug_assert_eq!(elf_header.pt2.type_().as_type(), header::Type::Executable);
    if elf_header.pt2.type_().as_type() != header::Type::Executable {
        return Err(ElfError::UnsupportedElfType);
    }

    Ok(())
}

#[derive(Debug)]
pub enum ElfError {
    FrameError(Error),
    NoSegment,
    UnsupportedElfType,
    SegmentNotPageAligned,
    UnreadableSegment,
    WithInfo(&'static str),
}

impl From<&'static str> for ElfError {
    fn from(error_info: &'static str) -> Self {
        ElfError::WithInfo(error_info)
    }
}

impl From<Error> for ElfError {
    fn from(frame_error: Error) -> Self {
        ElfError::FrameError(frame_error)
    }
}

fn read_segment_data<'a>(
    segment: ProgramHeader<'a>,
    elf_file: &ElfFile<'a>,
) -> Result<&'a [u8], &'static str> {
    match segment.get_data(&elf_file) {
        Err(msg) => Err(msg),
        Ok(data) => match data {
            SegmentData::Note64(_, data) | SegmentData::Undefined(data) => Ok(data),
            _ => Err("Unkonwn segment data type"),
        },
    }
}
