use core::ops::Range;

use crate::fs::file_handle::FileHandle;
use crate::fs::utils::SeekFrom;
use crate::prelude::*;
use crate::vm::vmar::{get_intersected_range, is_intersected};
use jinux_frame::vm::{VmAllocOptions, VmFrameVec, VmIo};
use jinux_frame::AlignExt;
use xmas_elf::program::ProgramHeader64;

use crate::vm::vmo::Pager;

// use super::load_elf::ElfSegment;

/// The pager behind a elf segment
pub struct ElfSegmentPager {
    /// The pager size
    pager_size: usize,
    /// the back up file
    file: Arc<FileHandle>,
    /// The segment offset in backup file
    file_offset: usize,
    /// The segment size in backup file
    file_size: usize,
    /// The offset for the segment data.
    /// The pager always starts at page-align address, while the segment data may start at any address.
    /// So the offset will be the segment data start address % PAGE_SIZE
    page_offset: usize,
}

impl ElfSegmentPager {
    pub fn new(file: Arc<FileHandle>, program_header: &ProgramHeader64) -> Self {
        let ph_start = program_header.virtual_addr as Vaddr;
        let ph_end = ph_start + program_header.mem_size as Vaddr;
        let start = ph_start.align_down(PAGE_SIZE);
        let end = ph_end.align_up(PAGE_SIZE);
        let pager_size = end - start;
        let offset = ph_start % PAGE_SIZE;
        Self {
            pager_size,
            file,
            file_offset: program_header.offset as usize,
            file_size: program_header.file_size as usize,
            page_offset: offset,
        }
    }
}

impl Pager for ElfSegmentPager {
    fn commit_page(&self, offset: usize) -> Result<jinux_frame::vm::VmFrame> {
        if offset >= self.pager_size {
            return_errno_with_message!(Errno::EINVAL, "offset exceeds pager size");
        }

        let vm_alloc_option = VmAllocOptions::new(1);
        let mut vm_frames = VmFrameVec::allocate(&vm_alloc_option)?;
        vm_frames.zero();

        let page_start = offset.align_down(PAGE_SIZE);
        let page_end = page_start + PAGE_SIZE;
        let page_range = page_start..page_end;
        let segment_range = self.page_offset..self.page_offset + self.file_size;
        if is_intersected(&page_range, &segment_range) {
            let intersected_range = get_intersected_range(&page_range, &segment_range);
            let segment_from_file_range = (intersected_range.start - self.page_offset)
                ..(intersected_range.end - self.page_offset);
            let mut segment_data = vec![0u8; segment_from_file_range.len()];
            self.file.seek(SeekFrom::Start(
                self.file_offset + segment_from_file_range.start,
            ))?;
            self.file.read(&mut segment_data)?;
            let write_offset = intersected_range.start % PAGE_SIZE;
            vm_frames.write_bytes(write_offset, &segment_data)?;
        }

        let vm_frame = vm_frames.pop().unwrap();
        Ok(vm_frame)
    }

    fn update_page(&self, offset: usize) -> Result<()> {
        unimplemented!()
    }

    fn decommit_page(&self, offset: usize) -> Result<()> {
        unimplemented!()
    }
}
