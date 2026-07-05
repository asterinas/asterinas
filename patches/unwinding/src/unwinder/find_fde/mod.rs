#[cfg(feature = "fde-custom")]
mod custom;
#[cfg(feature = "fde-static")]
mod fixed;
#[cfg(feature = "fde-gnu-eh-frame-hdr")]
mod gnu_eh_frame_hdr;
#[cfg(feature = "fde-phdr")]
mod phdr;
#[cfg(feature = "fde-registry")]
mod registry;

use crate::util::*;
use gimli::{BaseAddresses, EhFrame, FrameDescriptionEntry};

#[cfg(feature = "fde-custom")]
pub mod custom_eh_frame_finder {
    pub use super::custom::{
        EhFrameFinder, FrameInfo, FrameInfoKind, SetCustomEhFrameFinderError,
        set_custom_eh_frame_finder,
    };
}

#[derive(Debug)]
pub struct FDESearchResult {
    pub fde: FrameDescriptionEntry<StaticSlice>,
    pub bases: BaseAddresses,
    pub eh_frame: EhFrame<StaticSlice>,
}

pub trait FDEFinder {
    fn find_fde(&self, pc: usize) -> Option<FDESearchResult>;
}

pub struct GlobalFinder(());

impl FDEFinder for GlobalFinder {
    fn find_fde(&self, pc: usize) -> Option<FDESearchResult> {
        #[cfg(feature = "fde-custom")]
        if let Some(v) = custom::get_finder().find_fde(pc) {
            return Some(v);
        }
        #[cfg(feature = "fde-registry")]
        if let Some(v) = registry::get_finder().find_fde(pc) {
            return Some(v);
        }
        #[cfg(feature = "fde-gnu-eh-frame-hdr")]
        if let Some(v) = gnu_eh_frame_hdr::get_finder().find_fde(pc) {
            return Some(v);
        }
        #[cfg(feature = "fde-phdr")]
        if let Some(v) = phdr::get_finder().find_fde(pc) {
            return Some(v);
        }
        #[cfg(feature = "fde-static")]
        if let Some(v) = fixed::get_finder().find_fde(pc) {
            return Some(v);
        }
        None
    }
}

pub fn get_finder() -> &'static GlobalFinder {
    &GlobalFinder(())
}
