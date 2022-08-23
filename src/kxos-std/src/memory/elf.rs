use core::ops::Range;

use alloc::vec::Vec;
use kxos_frame::{
    vm::{Vaddr, VmAllocOptions, VmFrameVec, VmIo, VmPerm, VmSpace},
    Error,
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
        Ok(Self {
            range: start..end,
            type_,
            data,
        })
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

    pub fn map_self(&self, vm_space: &VmSpace, frames: VmFrameVec) -> Result<(), ElfError> {
        let mut vm_page_range = self.vm_page_range()?;
        let vm_perm = ElfLoadInfo::perm();
        vm_page_range.map_to(vm_space, frames, vm_perm);
        Ok(())
    }

    pub fn copy_elf(&self) -> Result<VmFrameVec, ElfError> {
        let vm_page_range = self.vm_page_range()?;
        // calculate offset
        let offset = vm_page_range.start_address();
        // allocate frames
        let page_number = vm_page_range.len();
        let options = VmAllocOptions::new(page_number);
        let mut frames = VmFrameVec::allocate(&options)?;

        for segment in self.segments.iter().filter(|segment| segment.is_loadable()) {
            let start_address = segment.start_address();
            frames.write_bytes(start_address - offset, segment.data)?;
        }

        Ok(frames)
    }

    pub fn map_and_clear_user_stack(&self, vm_space: &VmSpace) {
        self.user_stack.map_and_zeroed(vm_space);
    }

    /// return the perm of elf pages
    /// FIXME: Set the correct permission bit of user pages.
    fn perm() -> VmPerm {
        VmPerm::RX
    }

    pub fn entry_point(&self) -> u64 {
        self.entry_point as u64
    }

    pub fn user_stack_bottom(&self) -> u64 {
        self.user_stack.stack_bottom as u64
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
