// SPDX-License-Identifier: MPL-2.0

/// Copies `count * size_of::<T>()` bytes from `src` to `dst`.
/// The source and destination may overlap.
///
/// If the source and destination will never overlap, `fast_copy_nonoverlapping` can be used instead.
///
/// # Performance
///
/// This function is provided as a fast alternative to `core::ptr::copy` by
/// utilizing the CPU's `rep movsq` and `rep movsb` instructions for bulk memory copying.
/// These instructions can result in more efficient data transfers by moving larger blocks
/// of memory in a single operation, leading to fewer CPU cycles and better performance
/// in certain scenarios.
///
/// # Safety
///
/// The safety requirements of this function are consistent with `core::ptr::copy`.
#[inline]
pub unsafe fn fast_copy<T>(src: *const T, dst: *mut T, count: usize) {
    if src == dst || count == 0 {
        return;
    }

    if src < dst && src.add(count) > dst {
        // Overlap and src is before dst
        backward_copy(src, dst, count);
    } else {
        // No overlap, or src is after dst
        forward_copy(src, dst, count);
    }
}

/// Copies `count * size_of::<T>()` bytes from `src` to `dst`.
/// The source and destination must not overlap.
///
/// For regions of memory which might overlap, use `fast_copy` instead.
///
/// # Performance
///
/// This function is provided as a fast alternative to `core::ptr::copy_nonoverlapping` by
/// utilizing the CPU's `rep movsq` and `rep movsb` instructions for bulk memory copying.
/// These instructions can result in more efficient data transfers by moving larger blocks
/// of memory in a single operation, leading to fewer CPU cycles and better performance
/// in certain scenarios.
///
/// # Safety
///
/// The safety requirements of this function are consistent with `core::ptr::copy_nonoverlapping`.
#[inline]
pub unsafe fn fast_copy_nonoverlapping<T>(src: *const T, dst: *mut T, count: usize) {
    if count == 0 {
        return;
    }

    forward_copy(src, dst, count);
}

/// # Safety
///
/// The `src` and `dst` must point to valid memory regions.
/// If the memory regions of `src` and `dst` overlap, `src` must be higher than `dst`.
#[inline]
unsafe fn forward_copy<T>(src: *const T, dst: *mut T, count: usize) {
    let bytes_count = count * core::mem::size_of::<T>();

    // The direction of string copy instructions such as `rep movsb` is controlled by DF flag.
    // If `DF = 0`, then data copy is repeated from lower addresses to higher ones;
    // Otherwise, the data copy will be done in the reversed direction.
    // The System V ABI manual requires `DF = 0` on function entry
    // and all code before the `rep movsb` instruction in this function do not change DF flag.
    // Thus, we can safely assume `DF = 0`, which is exactly what we want.
    if bytes_count % 8 == 0 {
        // In most cases, `movsq` is faster than `movsb`
        // because it transfers larger chunks of data in a single operation.
        core::arch::asm!(
            "rep movsq",
            in("rcx") bytes_count / 8,
            in("rsi") src,
            in("rdi") dst,
            lateout("rcx") _,
            lateout("rsi") _,
            lateout("rdi") _
        );
    } else {
        core::arch::asm!(
            "rep movsb",
            in("rcx") bytes_count,
            in("rsi") src,
            in("rdi") dst,
            lateout("rcx") _,
            lateout("rsi") _,
            lateout("rdi") _
        );
    }
}

/// # Safety
///
/// The `src` and `dst` must point to valid memory regions.
/// If the memory regions of `src` and `dst` overlap, `src` must be lower than `dst`.
#[inline]
unsafe fn backward_copy<T>(src: *const T, dst: *mut T, count: usize) {
    let bytes_count = count * core::mem::size_of::<T>();
    let last_src = (src as *const u8).add(bytes_count).offset(-1);
    let last_dst = (dst as *mut u8).add(bytes_count).offset(-1);

    core::arch::asm!(
        "std", // Set the direction flag (DF)
        "rep movsb",
        in("rcx") bytes_count,
        in("rsi") last_src,
        in("rdi") last_dst,
        lateout("rcx") _,
        lateout("rsi") _,
        lateout("rdi") _
    );

    // System V ABI for AMD64 requires direction flag (DF) to be clear on function exit
    core::arch::asm!("cld");
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use super::*;
    #[ktest]
    fn test_fast_copy_nonoverlapping() {
        let src = vec![0u8; 8];
        let mut dst = vec![1u8; 8];

        unsafe {
            fast_copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr(), 8);
        }
        assert_eq!(src, dst);
    }

    #[ktest]
    fn test_fast_copy_src_after_dst() {
        let mut src = vec![0u8; 8];
        src.extend(vec![1u8; 8]);

        unsafe {
            fast_copy(src.as_ptr().add(4), src.as_mut_ptr(), 8);
        }

        let expected_left = {
            let mut vec = vec![0u8; 4];
            vec.extend(vec![1u8; 4]);
            vec
        };

        assert_eq!(expected_left, src[0..8]);
    }

    #[ktest]
    fn test_fast_copy_src_before_dst() {
        let mut src = vec![0u8; 8];
        src.extend(vec![1u8; 8]);

        unsafe {
            fast_copy(src.as_ptr().add(4), src.as_mut_ptr().add(8), 8);
        }

        let expected_right = {
            let mut vec = vec![0u8; 4];
            vec.extend(vec![1u8; 4]);
            vec
        };

        assert_eq!(expected_right, src[8..]);
    }
}
