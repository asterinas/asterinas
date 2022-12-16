use crate::prelude::*;
use crate::vm::vmar::{get_intersected_range, is_intersected};
use jinux_frame::vm::{VmAllocOptions, VmFrameVec, VmIo};
use jinux_frame::AlignExt;

use crate::vm::vmo::Pager;

use super::load_elf::ElfSegment;

/// The pager behind a elf segment
pub struct ElfSegmentPager {
    /// The pager size
    pager_size: usize,
    /// data for current segment
    segment_data: &'static [u8],
    /// The offset for the segment data.
    /// The pager always starts at page-align address, while the segment data may start at any address.
    /// So the offset will be the segment data start address % PAGE_SIZE
    offset: usize,
}

impl ElfSegmentPager {
    pub fn new(elf_file_content: &'static [u8], elf_segment: &ElfSegment) -> Self {
        let start = elf_segment.start_address().align_down(PAGE_SIZE);
        let end = elf_segment.end_address().align_up(PAGE_SIZE);
        let pager_size = end - start;
        let offset = elf_segment.start_address() % PAGE_SIZE;
        let elf_file_segment =
            &elf_file_content[elf_segment.offset..elf_segment.offset + elf_segment.file_size];
        Self {
            pager_size,
            segment_data: elf_file_segment,
            offset,
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
        let data_range = self.offset..self.offset + self.segment_data.len();
        if is_intersected(&page_range, &data_range) {
            let intersected_range = get_intersected_range(&page_range, &data_range);
            let data_write_range =
                (intersected_range.start - self.offset)..(intersected_range.end - self.offset);
            let write_content = &self.segment_data[data_write_range];
            let write_offset = intersected_range.start % PAGE_SIZE;
            vm_frames.write_bytes(write_offset, write_content)?;
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
