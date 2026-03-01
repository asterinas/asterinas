// SPDX-License-Identifier: MPL-2.0

use core::arch::asm;

use crate::{
    arch::{
        if_tdx_enabled,
        tdx_guest::{copy_from_mmio_slow, copy_to_mmio_slow},
    },
    mm::PodOnce,
};

/// Copies from MMIO to regular memory.
///
/// # Safety
///
/// - `dst` must be valid for writes of `count` bytes.
/// - `src` must be valid for MMIO reads of `count` bytes.
pub(crate) unsafe fn copy_from_mmio(dst: *mut u8, src: *const u8, count: usize) {
    if_tdx_enabled!({
        copy_from_mmio_slow(dst, src, count);
    } else {
        // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
        unsafe { copy_from_mmio_fast(dst, src, count) };
    });
}

/// Copies from regular memory to MMIO.
///
/// # Safety
///
/// - `src` must be valid for reads of `count` bytes.
/// - `dst` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn copy_to_mmio(src: *const u8, dst: *mut u8, count: usize) {
    if_tdx_enabled!({
        copy_to_mmio_slow(src, dst, count);
    } else {
        // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
        unsafe { copy_to_mmio_fast(src, dst, count) };
    });
}

/// Reads from a pointer with a non-tearing memory load.
///
/// This function is semantically equivalent to `core::ptr::read_volatile`
/// but is manually implemented with a single memory load instruction.
///
/// # Safety
///
/// Same as the safety requirement of `core::ptr::read_volatile`.
///
/// # Guarantee
///
/// The single-memory-load-instruction guarantee is particularly useful for
/// Confidential VMs (CVMs) such as Intel TDX and AMD SEV,
/// where the memory load may cause CPU exceptions (#VE and #VC, respectively)
/// and the kernel has to handle such exceptions
/// by decoding the faulting CPU instruction.
/// As such, the kernel must be compiled to emit simple load/store CPU instructions.
pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    let mut val: u64 = 0;

    // SAFETY: The caller guarantees `ptr` is valid for reads.
    unsafe {
        // EFFICIENCY: The match should be optimized out for release builds.
        //
        // This match is resolved at compile-time via monomorphization.
        // Since `size_of::<T>()` is a constant for each concrete instance of this
        // function, the compiler eliminates the branch and only emits the
        // `MOV` instruction for the matching size.
        match size_of::<T>() {
            1 => {
                asm!("mov {0:l}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            2 => {
                asm!("mov {0:x}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            4 => {
                asm!("mov {0:e}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            8 => {
                asm!("mov {0}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            _ => core::hint::unreachable_unchecked(),
        }
        // EFFICIENCY: This should compile to a no-op for release builds.
        // This is because both the source and destination locations fit in a register.
        // So it only _re-interprets_ bits and no copying is needed.
        core::ptr::read((&val as *const u64).cast::<T>())
    }
}

/// Writes to a pointer with a non-tearing memory store.
///
/// # Safety
///
/// Same as the safety requirement of `core::ptr::write_volatile`.
///
/// # Guarantee
///
/// Refer to the "Guarantee" section of [`read_once`].
pub(crate) unsafe fn write_once<T: PodOnce>(ptr: *mut T, val: T) {
    let mut tmp: u64 = 0;

    // SAFETY: The caller guarantees `ptr` is valid for writes.
    unsafe {
        // EFFICIENCY: This should be a no-op for release build.
        // This is because both the source and destination locations fit in a register.
        // So it only _re-interprets_ bits and no copying is needed.
        core::ptr::write((&mut tmp as *mut u64).cast::<T>(), val);

        // EFFICIENCY: The match here has no overhead for release build.
        //
        // This match is resolved at compile-time via monomorphization.
        // Since `size_of::<T>()` is a constant for each concrete instance of this
        // function, the compiler eliminates the branch and only emits the
        // `MOV` instruction for the matching size.
        match size_of::<T>() {
            1 => {
                asm!("mov [{0}], {1:l}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            2 => {
                asm!("mov [{0}], {1:x}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            4 => {
                asm!("mov [{0}], {1:e}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            8 => {
                asm!("mov [{0}], {1}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            _ => core::hint::unreachable_unchecked(),
        }
    }
}

/// Copies one byte from MMIO to regular memory and advances both pointers.
///
/// # Safety
///
/// - `src_io_ptr` must be valid for one byte of MMIO read.
/// - `dst_ptr` must be valid for one byte of regular memory write.
pub(crate) unsafe fn copy_u8_from(dst_ptr: &mut *mut u8, src_io_ptr: &mut *const u8) {
    // SAFETY: The caller guarantees the MMIO and regular memory pointers are valid.
    unsafe { copy_from_mmio_val::<u8>(dst_ptr, src_io_ptr) }
}

/// Copies a u16 from MMIO to regular memory and advances both pointers.
///
/// # Safety
///
/// - `src_io_ptr` must be aligned and valid for a u16 MMIO read.
/// - `dst_ptr` must be valid for a u16 write.
pub(crate) unsafe fn copy_u16_from(dst_ptr: &mut *mut u8, src_io_ptr: &mut *const u8) {
    debug_assert!(src_io_ptr.addr() & 1 == 0);
    // SAFETY: The caller guarantees the MMIO and regular memory pointers are valid.
    unsafe { copy_from_mmio_val::<u16>(dst_ptr, src_io_ptr) }
}

/// Copies a u32 from MMIO to regular memory and advances both pointers.
///
/// # Safety
///
/// - `src_io_ptr` must be aligned and valid for a u32 MMIO read.
/// - `dst_ptr` must be valid for a u32 write.
pub(crate) unsafe fn copy_u32_from(dst_ptr: &mut *mut u8, src_io_ptr: &mut *const u8) {
    debug_assert!(src_io_ptr.addr() & 3 == 0);
    // SAFETY: The caller guarantees the MMIO and regular memory pointers are valid.
    unsafe { copy_from_mmio_val::<u32>(dst_ptr, src_io_ptr) }
}

/// Copies a u64 from MMIO to regular memory and advances both pointers.
///
/// # Safety
///
/// - `src_io_ptr` must be aligned and valid for a u64 MMIO read.
/// - `dst_ptr` must be valid for a u64 write.
pub(crate) unsafe fn copy_u64_from(dst_ptr: &mut *mut u8, src_io_ptr: &mut *const u8) {
    debug_assert!(src_io_ptr.addr() & 7 == 0);
    // SAFETY: The caller guarantees the MMIO and regular memory pointers are valid.
    unsafe { copy_from_mmio_val::<u64>(dst_ptr, src_io_ptr) }
}

/// Copies one byte from regular memory to MMIO and advances both pointers.
///
/// # Safety
///
/// - `src_ptr` must be valid for one byte of regular memory read.
/// - `dst_io_ptr` must be valid for one byte of MMIO write.
pub(crate) unsafe fn copy_u8_to(src_ptr: &mut *const u8, dst_io_ptr: &mut *mut u8) {
    // SAFETY: The caller guarantees the regular memory and MMIO pointers are valid.
    unsafe { copy_to_mmio_val::<u8>(src_ptr, dst_io_ptr) }
}

/// Copies a u16 from regular memory to MMIO and advances both pointers.
///
/// # Safety
///
/// - `src_ptr` must be valid for a u16 read.
/// - `dst_io_ptr` must be aligned and valid for a u16 MMIO write.
pub(crate) unsafe fn copy_u16_to(src_ptr: &mut *const u8, dst_io_ptr: &mut *mut u8) {
    debug_assert!(dst_io_ptr.addr() & 1 == 0);
    // SAFETY: The caller guarantees the regular memory and MMIO pointers are valid.
    unsafe { copy_to_mmio_val::<u16>(src_ptr, dst_io_ptr) }
}

/// Copies a u32 from regular memory to MMIO and advances both pointers.
///
/// # Safety
///
/// - `src_ptr` must be valid for a u32 read.
/// - `dst_io_ptr` must be aligned and valid for a u32 MMIO write.
pub(crate) unsafe fn copy_u32_to(src_ptr: &mut *const u8, dst_io_ptr: &mut *mut u8) {
    debug_assert!(dst_io_ptr.addr() & 3 == 0);
    // SAFETY: The caller guarantees the regular memory and MMIO pointers are valid.
    unsafe { copy_to_mmio_val::<u32>(src_ptr, dst_io_ptr) }
}

/// Copies a u64 from regular memory to MMIO and advances both pointers.
///
/// # Safety
///
/// - `src_ptr` must be valid for a u64 read.
/// - `dst_io_ptr` must be aligned and valid for a u64 MMIO write.
pub(crate) unsafe fn copy_u64_to(src_ptr: &mut *const u8, dst_io_ptr: &mut *mut u8) {
    debug_assert!(dst_io_ptr.addr() & 7 == 0);
    // SAFETY: The caller guarantees the regular memory and MMIO pointers are valid.
    unsafe { copy_to_mmio_val::<u64>(src_ptr, dst_io_ptr) }
}

unsafe fn copy_from_mmio_val<T: PodOnce>(dst_ptr: &mut *mut u8, src_io_ptr: &mut *const u8) {
    // SAFETY: The caller guarantees the MMIO and regular memory pointers are valid.
    unsafe {
        let val: T = read_once((*src_io_ptr).cast::<T>());
        let dst = *dst_ptr;
        let align_mask = size_of::<T>() - 1;
        if dst.addr() & align_mask == 0 {
            core::ptr::write(dst.cast::<T>(), val);
        } else {
            core::ptr::write_unaligned(dst.cast::<T>(), val);
        }
        *src_io_ptr = (*src_io_ptr).add(size_of::<T>());
        *dst_ptr = dst.add(size_of::<T>());
    }
}

unsafe fn copy_to_mmio_val<T: PodOnce>(src_ptr: &mut *const u8, dst_io_ptr: &mut *mut u8) {
    // SAFETY: The caller guarantees the regular memory and MMIO pointers are valid.
    unsafe {
        let src = *src_ptr;
        let align_mask = size_of::<T>() - 1;
        let val: T = if src.addr() & align_mask != 0 {
            core::ptr::read_unaligned(src.cast::<T>())
        } else {
            core::ptr::read(src.cast::<T>())
        };
        write_once((*dst_io_ptr).cast::<T>(), val);
        *src_ptr = src.add(size_of::<T>());
        *dst_io_ptr = (*dst_io_ptr).add(size_of::<T>());
    }
}

/// Copies from MMIO to regular memory using string move instructions.
///
/// # Safety
///
/// - `src` must be valid for MMIO reads of `count` bytes.
/// - `dst` must be valid for writes of `count` bytes.
unsafe fn copy_from_mmio_fast(mut dst: *mut u8, mut src: *const u8, mut count: usize) {
    if count == 0 {
        return;
    }

    // Align source IO to 2 bytes.
    if src.addr() & 1 != 0 {
        // SAFETY: The caller guarantees both pointers are valid.
        unsafe {
            asm!("movsb", inout("rdi") dst, inout("rsi") src, options(nostack, preserves_flags));
        }
        count -= 1;
    }

    if count >= 2 && src.addr() & 2 != 0 {
        // SAFETY: The caller guarantees both pointers are valid for 2 bytes.
        unsafe {
            asm!("movsw", inout("rdi") dst, inout("rsi") src, options(nostack, preserves_flags));
        }
        count -= 2;
    }

    if count > 0 {
        // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
        unsafe {
            asm!(
                "rep movsb",
                inout("rdi") dst,
                inout("rsi") src,
                inout("rcx") count,
                options(nostack, preserves_flags)
            );
        }
        let _ = (dst, src, count);
    }
}

/// Copies from regular memory to MMIO using string move instructions.
///
/// # Safety
///
/// - `src` must be valid for reads of `count` bytes.
/// - `dst` must be valid for MMIO writes of `count` bytes.
unsafe fn copy_to_mmio_fast(mut src: *const u8, mut dst: *mut u8, mut count: usize) {
    if count == 0 {
        return;
    }

    // Align destination IO to 2 bytes.
    if dst.addr() & 1 != 0 {
        // SAFETY: The caller guarantees both pointers are valid.
        unsafe {
            asm!("movsb", inout("rdi") dst, inout("rsi") src, options(nostack, preserves_flags));
        }
        count -= 1;
    }

    if count >= 2 && dst.addr() & 2 != 0 {
        // SAFETY: The caller guarantees both pointers are valid for 2 bytes.
        unsafe {
            asm!("movsw", inout("rdi") dst, inout("rsi") src, options(nostack, preserves_flags));
        }
        count -= 2;
    }

    if count > 0 {
        // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
        unsafe {
            asm!(
                "rep movsb",
                inout("rdi") dst,
                inout("rsi") src,
                inout("rcx") count,
                options(nostack, preserves_flags)
            );
        }
        let _ = (dst, src, count);
    }
}
