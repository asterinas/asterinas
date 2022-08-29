use core::{ops::Range, cmp::Ordering};

use alloc::vec::Vec;
use alloc::vec;
use kxos_frame::{
    vm::{Vaddr, VmAllocOptions, VmFrameVec, VmIo, VmPerm, VmSpace, VmMapOptions},
    Error, config::PAGE_SIZE,
};
use xmas_elf::{
    header,
    program::{self, ProgramHeader, SegmentData},
    ElfFile,
};

use super::{user_stack::UserStack, vm_page::VmPageRange};

pub struct ElfLoadInfo<'a> {
    entry_point: Vaddr,
    segments: Vec<ElfSegment<'a>>,
    user_stack: UserStack,
}

pub struct ElfSegment<'a> {
    range: Range<Vaddr>,
    data: &'a [u8],
    type_: program::Type,
    vm_perm: VmPerm,
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
            Err(_) => return Err(ElfError::from("")),
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

    fn copy_and_map(&self, vm_space: &VmSpace) -> Result<(), ElfError> {
        if !self.is_page_aligned() {
            return Err(ElfError::SegmentNotPageAligned);
        }
        let vm_page_range = VmPageRange::new_range(self.start_address()..self.end_address());
        let page_number = vm_page_range.len();
        // allocate frames
        let vm_alloc_options = VmAllocOptions::new(page_number);
        let mut frames = VmFrameVec::allocate(&vm_alloc_options)?;
        // copy segment
        frames.write_bytes(0, self.data)?;
        // map segment
        let mut vm_map_options = VmMapOptions::new();
        vm_map_options.addr(Some(self.start_address()));
        vm_map_options.perm(self.vm_perm);
        vm_space.map(frames, &vm_map_options)?;
        Ok(())
    }

    fn is_page_aligned(&self) -> bool {
        self.start_address() % PAGE_SIZE == 0
    }
}

impl<'a> ElfLoadInfo<'a> {
    fn with_capacity(entry_point: Vaddr, capacity: usize, user_stack: UserStack) -> Self {
        Self {
            entry_point,
            segments: Vec::with_capacity(capacity),
            user_stack,
        }
    }

    fn add_segment(&mut self, elf_segment: ElfSegment<'a>) {
        self.segments.push(elf_segment);
    }

    pub fn parse_elf_data(elf_file_content: &'a [u8]) -> Result<Self, ElfError> {
        let elf_file = match ElfFile::new(elf_file_content) {
            Err(error_msg) => return Err(ElfError::from(error_msg)),
            Ok(elf_file) => elf_file,
        };
        check_elf_header(&elf_file)?;
        // init elf load info
        let entry_point = elf_file.header.pt2.entry_point() as Vaddr;
        // FIXME: only contains load segment?
        let segments_count = elf_file.program_iter().count();
        let user_stack = UserStack::new_default_config();
        let mut elf_load_info = ElfLoadInfo::with_capacity(entry_point, segments_count, user_stack);

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

    pub fn copy_and_map(&self, vm_space: &VmSpace) -> Result<(), ElfError> {
        for segment in self.segments.iter() {
            segment.copy_and_map(vm_space)?;
        }
        Ok(())
    }

    pub fn map_and_clear_user_stack(&self, vm_space: &VmSpace) {
        self.user_stack.map_and_zeroed(vm_space);
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
        self.user_stack.stack_top() as u64
    }

    /// read content from vmspace to ensure elf data is correctly copied to user space
    pub fn debug_check_map_result(&self, vm_space: &VmSpace) {
        for segment in self.segments.iter() {
            let start_address = segment.start_address();
            let len = segment.data.len();
            let mut read_buffer = vec![0;len];
            vm_space.read_bytes(start_address, &mut read_buffer).expect("read bytes failed");
            let res = segment.data.cmp(&read_buffer);
            assert_eq!(res, Ordering::Equal);
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
    debug_assert_eq!(elf_header.pt1.os_abi(), header::OsAbi::SystemV);
    if elf_header.pt1.os_abi() != header::OsAbi::SystemV {
        return Err(ElfError::UnsupportedElfType);
    }
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
) -> Result<&'a [u8], ()> {
    match segment.get_data(&elf_file) {
        Err(_) => Err(()),
        Ok(data) => {
            if let SegmentData::Undefined(data) = data {
                Ok(data)
            } else {
                Err(())
            }
        }
    }
}
