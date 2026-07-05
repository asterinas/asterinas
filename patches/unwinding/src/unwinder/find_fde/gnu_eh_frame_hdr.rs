use super::FDESearchResult;
use crate::util::*;

use gimli::{BaseAddresses, EhFrame, EhFrameHdr, NativeEndian, UnwindSection};

pub struct StaticFinder(());

pub fn get_finder() -> &'static StaticFinder {
    &StaticFinder(())
}

unsafe extern "C" {
    static __executable_start: u8;
    static __etext: u8;
    static __GNU_EH_FRAME_HDR: u8;
}

impl super::FDEFinder for StaticFinder {
    fn find_fde(&self, pc: usize) -> Option<FDESearchResult> {
        unsafe {
            let text_start = &__executable_start as *const u8 as usize;
            let text_end = &__etext as *const u8 as usize;
            if !(text_start..text_end).contains(&pc) {
                return None;
            }

            let eh_frame_hdr = &__GNU_EH_FRAME_HDR as *const u8 as usize;
            let bases = BaseAddresses::default()
                .set_text(text_start as _)
                .set_eh_frame_hdr(eh_frame_hdr as _);
            let eh_frame_hdr =
                EhFrameHdr::new(get_unlimited_slice(eh_frame_hdr as _), NativeEndian)
                    .parse(&bases, core::mem::size_of::<usize>() as _)
                    .ok()?;
            let eh_frame = deref_pointer(eh_frame_hdr.eh_frame_ptr());
            let bases = bases.set_eh_frame(eh_frame as _);
            let eh_frame = EhFrame::new(get_unlimited_slice(eh_frame as _), NativeEndian);

            // Use binary search table for address if available.
            if let Some(table) = eh_frame_hdr.table()
                && let Ok(fde) =
                    table.fde_for_address(&eh_frame, &bases, pc as _, EhFrame::cie_from_offset)
            {
                return Some(FDESearchResult {
                    fde,
                    bases,
                    eh_frame,
                });
            }

            // Otherwise do the linear search.
            if let Ok(fde) = eh_frame.fde_for_address(&bases, pc as _, EhFrame::cie_from_offset) {
                return Some(FDESearchResult {
                    fde,
                    bases,
                    eh_frame,
                });
            }

            None
        }
    }
}
