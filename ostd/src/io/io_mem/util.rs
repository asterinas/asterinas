// SPDX-License-Identifier: MPL-2.0

use crate::arch::io::io_mem::{read_once, write_once};

/// Copies from I/O memory to regular memory.
///
/// This avoids `rep movs` so the CPU emits simple load/store instructions,
/// which is required on some platforms (e.g., TDX).
pub unsafe fn copy_from(mut dst: *mut u8, mut src: *const u8, mut count: usize) {
    let word_size = core::mem::size_of::<usize>();

    while count > 0 && !(src as usize).is_multiple_of(word_size) {
        unsafe {
            let val: u8 = read_once(src);
            core::ptr::write(dst, val);
            src = src.add(1);
            dst = dst.add(1);
        }
        count -= 1;
    }

    while count >= word_size {
        unsafe {
            let val: usize = read_once(src as *const usize);
            core::ptr::write_unaligned(dst as *mut usize, val);
            src = src.add(word_size);
            dst = dst.add(word_size);
        }
        count -= word_size;
    }

    while count > 0 {
        unsafe {
            let val: u8 = read_once(src);
            core::ptr::write(dst, val);
            src = src.add(1);
            dst = dst.add(1);
        }
        count -= 1;
    }
}

/// Copies from regular memory to I/O memory.
pub unsafe fn copy_to(mut dst: *mut u8, mut src: *const u8, mut count: usize) {
    let word_size = core::mem::size_of::<usize>();

    while count > 0 && !(dst as usize).is_multiple_of(word_size) {
        unsafe {
            let val: u8 = core::ptr::read(src);
            write_once(dst, val);
            src = src.add(1);
            dst = dst.add(1);
        }
        count -= 1;
    }

    while count >= word_size {
        unsafe {
            let val: usize = core::ptr::read_unaligned(src as *const usize);
            write_once(dst as *mut usize, val);
            src = src.add(word_size);
            dst = dst.add(word_size);
        }
        count -= word_size;
    }

    while count > 0 {
        unsafe {
            let val: u8 = core::ptr::read(src);
            write_once(dst, val);
            src = src.add(1);
            dst = dst.add(1);
        }
        count -= 1;
    }
}

#[cfg(ktest)]
mod test_read_once_and_write_once {
    use super::{read_once, write_once};
    use crate::prelude::ktest;

    #[ktest]
    fn read_write_u8() {
        let mut data: u8 = 0;
        unsafe {
            write_once(&mut data, 42u8);
            assert_eq!(read_once(&data), 42u8);
        }
    }

    #[ktest]
    fn read_write_u16() {
        let mut data: u16 = 0;
        let val: u16 = 0x1234;
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn read_write_u32() {
        let mut data: u32 = 0;
        let val: u32 = 0x12345678;
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn read_write_u64() {
        let mut data: u64 = 0;
        let val: u64 = 0xDEADBEEFCAFEBABE;
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn test_boundary_overlap() {
        let mut data: [u8; 2] = [0xAA, 0xBB];
        unsafe {
            write_once(&mut data[0], 0x11u8);
            assert_eq!(data[0], 0x11);
            assert_eq!(data[1], 0xBB);
        }
    }
}

#[cfg(ktest)]
mod test_copy_helpers {
    use super::{copy_from, copy_to};
    use crate::prelude::ktest;

    fn fill_pattern(buf: &mut [u8]) {
        for (idx, byte) in buf.iter_mut().enumerate() {
            *byte = (idx as u8).wrapping_mul(3).wrapping_add(1);
        }
    }

    fn run_copy_from_case(src_offset: usize, dst_offset: usize, len: usize) {
        let mut src = [0u8; 64];
        let mut dst = [0u8; 64];
        fill_pattern(&mut src);

        let src_ptr = unsafe { src.as_ptr().add(src_offset) };
        let dst_ptr = unsafe { dst.as_mut_ptr().add(dst_offset) };

        unsafe { copy_from(dst_ptr, src_ptr, len) };

        assert_eq!(
            &dst[dst_offset..dst_offset + len],
            &src[src_offset..src_offset + len]
        );
    }

    fn run_copy_to_case(src_offset: usize, dst_offset: usize, len: usize) {
        let mut src = [0u8; 64];
        let mut dst = [0u8; 64];
        fill_pattern(&mut src);

        let src_ptr = unsafe { src.as_ptr().add(src_offset) };
        let dst_ptr = unsafe { dst.as_mut_ptr().add(dst_offset) };

        unsafe { copy_to(dst_ptr, src_ptr, len) };

        assert_eq!(
            &dst[dst_offset..dst_offset + len],
            &src[src_offset..src_offset + len]
        );
    }

    #[ktest]
    fn copy_from_alignment_and_sizes() {
        let word_size = core::mem::size_of::<usize>();
        let sizes = [
            0,
            1,
            word_size.saturating_sub(1),
            word_size,
            word_size + 1,
            word_size * 2 + 3,
        ];
        let offsets = [0, 1, 2];

        for &len in &sizes {
            for &src_offset in &offsets {
                for &dst_offset in &offsets {
                    if src_offset + len <= 64 && dst_offset + len <= 64 {
                        run_copy_from_case(src_offset, dst_offset, len);
                    }
                }
            }
        }
    }

    #[ktest]
    fn copy_to_alignment_and_sizes() {
        let word_size = core::mem::size_of::<usize>();
        let sizes = [
            0,
            1,
            word_size.saturating_sub(1),
            word_size,
            word_size + 1,
            word_size * 2 + 3,
        ];
        let offsets = [0, 1, 2];

        for &len in &sizes {
            for &src_offset in &offsets {
                for &dst_offset in &offsets {
                    if src_offset + len <= 64 && dst_offset + len <= 64 {
                        run_copy_to_case(src_offset, dst_offset, len);
                    }
                }
            }
        }
    }
}
