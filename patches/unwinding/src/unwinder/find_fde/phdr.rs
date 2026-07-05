use super::FDESearchResult;
use crate::util::*;

use core::mem;
use core::slice;
use gimli::{BaseAddresses, EhFrame, EhFrameHdr, NativeEndian, UnwindSection};
use libc::{PT_DYNAMIC, PT_GNU_EH_FRAME, PT_LOAD};

#[cfg(target_pointer_width = "32")]
use libc::Elf32_Phdr as Elf_Phdr;
#[cfg(target_pointer_width = "64")]
use libc::Elf64_Phdr as Elf_Phdr;

pub struct PhdrFinder(());

pub fn get_finder() -> &'static PhdrFinder {
    &PhdrFinder(())
}

impl super::FDEFinder for PhdrFinder {
    fn find_fde(&self, pc: usize) -> Option<FDESearchResult> {
        #[cfg(feature = "fde-phdr-aux")]
        if let Some(v) = search_aux_phdr(pc) {
            return Some(v);
        }
        #[cfg(feature = "fde-phdr-dl")]
        if let Some(v) = search_dl_phdr(pc) {
            return Some(v);
        }
        None
    }
}

#[cfg(feature = "fde-phdr-aux")]
fn search_aux_phdr(pc: usize) -> Option<FDESearchResult> {
    use libc::{AT_PHDR, AT_PHNUM, PT_PHDR, getauxval};

    unsafe {
        let phdr = getauxval(AT_PHDR) as *const Elf_Phdr;
        let phnum = getauxval(AT_PHNUM) as usize;
        let phdrs = slice::from_raw_parts(phdr, phnum);
        // With known address of PHDR, we can calculate the base address in reverse.
        let base =
            phdrs.as_ptr() as usize - phdrs.iter().find(|x| x.p_type == PT_PHDR)?.p_vaddr as usize;
        search_phdr(phdrs, base, pc)
    }
}

#[cfg(feature = "fde-phdr-dl")]
fn search_dl_phdr(pc: usize) -> Option<FDESearchResult> {
    use core::ffi::c_void;
    use libc::{dl_iterate_phdr, dl_phdr_info};

    struct CallbackData {
        pc: usize,
        result: Option<FDESearchResult>,
    }

    unsafe extern "C" fn phdr_callback(
        info: *mut dl_phdr_info,
        _size: usize,
        data: *mut c_void,
    ) -> c_int {
        unsafe {
            let data = &mut *(data as *mut CallbackData);
            let phdrs = slice::from_raw_parts((*info).dlpi_phdr, (*info).dlpi_phnum as usize);
            if let Some(v) = search_phdr(phdrs, (*info).dlpi_addr as _, data.pc) {
                data.result = Some(v);
                return 1;
            }
            0
        }
    }

    let mut data = CallbackData { pc, result: None };
    unsafe { dl_iterate_phdr(Some(phdr_callback), &mut data as *mut CallbackData as _) };
    data.result
}

fn search_phdr(phdrs: &[Elf_Phdr], base: usize, pc: usize) -> Option<FDESearchResult> {
    unsafe {
        let mut text = None;
        let mut eh_frame_hdr = None;
        let mut dynamic = None;

        for phdr in phdrs {
            let start = base + phdr.p_vaddr as usize;
            match phdr.p_type {
                PT_LOAD => {
                    let end = start + phdr.p_memsz as usize;
                    let range = start..end;
                    if range.contains(&pc) {
                        text = Some(range);
                    }
                }
                PT_GNU_EH_FRAME => {
                    eh_frame_hdr = Some(start);
                }
                PT_DYNAMIC => {
                    dynamic = Some(start);
                }
                _ => (),
            }
        }

        let text = text?;
        let eh_frame_hdr = eh_frame_hdr?;

        let mut bases = BaseAddresses::default()
            .set_eh_frame_hdr(eh_frame_hdr as _)
            .set_text(text.start as _);

        // Find the GOT section.
        if let Some(start) = dynamic {
            const DT_NULL: usize = 0;
            const DT_PLTGOT: usize = 3;

            let mut tags = start as *const [usize; 2];
            let mut tag = *tags;
            while tag[0] != DT_NULL {
                if tag[0] == DT_PLTGOT {
                    bases = bases.set_got(tag[1] as _);
                    break;
                }
                tags = tags.add(1);
                tag = *tags;
            }
        }

        // Parse .eh_frame_hdr section.
        let eh_frame_hdr = EhFrameHdr::new(get_unlimited_slice(eh_frame_hdr as _), NativeEndian)
            .parse(&bases, mem::size_of::<usize>() as _)
            .ok()?;

        let eh_frame = deref_pointer(eh_frame_hdr.eh_frame_ptr());
        bases = bases.set_eh_frame(eh_frame as _);
        let eh_frame = EhFrame::new(get_unlimited_slice(eh_frame as usize as _), NativeEndian);

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
