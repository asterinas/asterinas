use super::{FDEFinder, FDESearchResult};
use crate::util::{deref_pointer, get_unlimited_slice};

use core::sync::atomic::{AtomicU32, Ordering};
use gimli::{BaseAddresses, EhFrame, EhFrameHdr, NativeEndian, UnwindSection};

pub(crate) struct CustomFinder(());

pub(crate) fn get_finder() -> &'static CustomFinder {
    &CustomFinder(())
}

impl FDEFinder for CustomFinder {
    fn find_fde(&self, pc: usize) -> Option<FDESearchResult> {
        get_custom_eh_frame_finder().and_then(|eh_frame_finder| find_fde(eh_frame_finder, pc))
    }
}

/// A trait for types whose values can be used as the global EH frame finder set by [`set_custom_eh_frame_finder`].
pub unsafe trait EhFrameFinder {
    fn find(&self, pc: usize) -> Option<FrameInfo>;
}

pub struct FrameInfo {
    pub text_base: Option<usize>,
    pub kind: FrameInfoKind,
}

pub enum FrameInfoKind {
    EhFrameHdr(usize),
    EhFrame(usize),
}

static mut CUSTOM_EH_FRAME_FINDER: Option<&(dyn EhFrameFinder + Sync)> = None;

static CUSTOM_EH_FRAME_FINDER_STATE: AtomicU32 = AtomicU32::new(UNINITIALIZED);

const UNINITIALIZED: u32 = 0;
const INITIALIZING: u32 = 1;
const INITIALIZED: u32 = 2;

/// The type returned by [`set_custom_eh_frame_finder`] if [`set_custom_eh_frame_finder`] has
/// already been called.
#[derive(Debug)]
pub struct SetCustomEhFrameFinderError(());

/// Sets the global EH frame finder.
///
/// This function should only be called once during the lifetime of the program.
///
/// # Errors
///
/// An error is returned if this function has already been called during the lifetime of the
/// program.
pub fn set_custom_eh_frame_finder(
    fde_finder: &'static (dyn EhFrameFinder + Sync),
) -> Result<(), SetCustomEhFrameFinderError> {
    match CUSTOM_EH_FRAME_FINDER_STATE.compare_exchange(
        UNINITIALIZED,
        INITIALIZING,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(UNINITIALIZED) => {
            unsafe {
                CUSTOM_EH_FRAME_FINDER = Some(fde_finder);
            }
            CUSTOM_EH_FRAME_FINDER_STATE.store(INITIALIZED, Ordering::SeqCst);
            Ok(())
        }
        Err(INITIALIZING) => {
            while CUSTOM_EH_FRAME_FINDER_STATE.load(Ordering::SeqCst) == INITIALIZING {
                core::hint::spin_loop();
            }
            Err(SetCustomEhFrameFinderError(()))
        }
        Err(INITIALIZED) => Err(SetCustomEhFrameFinderError(())),
        _ => {
            unreachable!()
        }
    }
}

fn get_custom_eh_frame_finder() -> Option<&'static dyn EhFrameFinder> {
    if CUSTOM_EH_FRAME_FINDER_STATE.load(Ordering::SeqCst) == INITIALIZED {
        Some(unsafe { CUSTOM_EH_FRAME_FINDER.unwrap() })
    } else {
        None
    }
}

fn find_fde<T: EhFrameFinder + ?Sized>(eh_frame_finder: &T, pc: usize) -> Option<FDESearchResult> {
    let info = eh_frame_finder.find(pc)?;
    let text_base = info.text_base;
    match info.kind {
        FrameInfoKind::EhFrameHdr(eh_frame_hdr) => {
            find_fde_with_eh_frame_hdr(pc, text_base, eh_frame_hdr)
        }
        FrameInfoKind::EhFrame(eh_frame) => find_fde_with_eh_frame(pc, text_base, eh_frame),
    }
}

fn find_fde_with_eh_frame_hdr(
    pc: usize,
    text_base: Option<usize>,
    eh_frame_hdr: usize,
) -> Option<FDESearchResult> {
    unsafe {
        let mut bases = BaseAddresses::default().set_eh_frame_hdr(eh_frame_hdr as _);
        if let Some(text_base) = text_base {
            bases = bases.set_text(text_base as _);
        }
        let eh_frame_hdr = EhFrameHdr::new(get_unlimited_slice(eh_frame_hdr as _), NativeEndian)
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

fn find_fde_with_eh_frame(
    pc: usize,
    text_base: Option<usize>,
    eh_frame: usize,
) -> Option<FDESearchResult> {
    unsafe {
        let mut bases = BaseAddresses::default().set_eh_frame(eh_frame as _);
        if let Some(text_base) = text_base {
            bases = bases.set_text(text_base as _);
        }
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
