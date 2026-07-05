use super::FDESearchResult;
use crate::util::*;

use gimli::{BaseAddresses, EhFrame, NativeEndian, UnwindSection};

pub struct StaticFinder(());

pub fn get_finder() -> &'static StaticFinder {
    &StaticFinder(())
}

unsafe extern "C" {
    static __executable_start: u8;
    static __etext: u8;
    static __eh_frame: u8;
}

impl super::FDEFinder for StaticFinder {
    fn find_fde(&self, pc: usize) -> Option<FDESearchResult> {
        unsafe {
            let text_start = &__executable_start as *const u8 as usize;
            let text_end = &__etext as *const u8 as usize;
            if !(text_start..text_end).contains(&pc) {
                return None;
            }

            let eh_frame = &__eh_frame as *const u8 as usize;
            let bases = BaseAddresses::default()
                .set_eh_frame(eh_frame as _)
                .set_text(text_start as _);
            let eh_frame = EhFrame::new(get_unlimited_slice(eh_frame as _), NativeEndian);

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
