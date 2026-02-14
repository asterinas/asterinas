// SPDX-License-Identifier: MPL-2.0

use crate::{
    Error,
    arch::io::io_mem::{copy_from_mmio, copy_to_mmio},
    mm::{FallibleVmRead, FallibleVmWrite, VmReader, VmWriter, io::memcpy_fallible},
};

/// Copies from I/O memory to regular memory.
///
/// This uses simple load/store instructions, which is required on some platforms
/// (e.g., TDX).
///
/// # Safety
///
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
/// - `dst_ptr` must be valid for writes of `count` bytes.
pub(crate) unsafe fn copy_from_io_mem(dst_ptr: *mut u8, src_io_ptr: *const u8, count: usize) {
    // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
    unsafe { copy_from_mmio(dst_ptr, src_io_ptr, count) };
}

/// Copies from regular memory to I/O memory.
///
/// # Safety
///
/// - `src_ptr` must be valid for reads of `count` bytes.
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn copy_to_io_mem(src_ptr: *const u8, dst_io_ptr: *mut u8, count: usize) {
    // SAFETY: The caller guarantees both pointers are valid for `count` bytes.
    unsafe { copy_to_mmio(src_ptr, dst_io_ptr, count) };
}

/// Copies from I/O memory to regular memory with fallible destination writes.
///
/// The fallback path uses a bounce buffer when a direct MMIO-to-user copy
/// is unavailable. `VmIo` does not support short reads, so failures return
/// immediately instead of continuing with partial data.
///
/// # Safety
///
/// - `src_io_ptr` must be valid for MMIO reads of `count` bytes.
pub(crate) unsafe fn copy_from_io_to_writer(
    writer: &mut VmWriter,
    mut src_io_ptr: *const u8,
    mut count: usize,
) -> Result<(), (Error, usize)> {
    if let Some(result) = unsafe { try_copy_io_to_writer_direct(writer, src_io_ptr, count) } {
        return result;
    }

    // Fallback: direct MMIO->user copy unavailable; use bounce buffer.
    const BUF_SIZE: usize = 4096;
    let mut buf = [0u8; BUF_SIZE];
    let mut copied_total = 0;

    while count > 0 {
        let chunk = core::cmp::min(count, core::cmp::min(buf.len(), writer.avail()));

        if chunk == 0 {
            return Err((Error::PageFault, copied_total));
        }

        // SAFETY: The caller guarantees the MMIO range is valid for this chunk.
        unsafe { copy_from_io_mem(buf.as_mut_ptr(), src_io_ptr, chunk) };

        let mut reader = VmReader::from(&buf[..chunk]);
        match writer.write_fallible(&mut reader) {
            Ok(written) => {
                copied_total += written;
                if written < chunk {
                    return Err((Error::PageFault, copied_total));
                }
            }
            Err((err, written)) => {
                copied_total += written;
                return Err((err, copied_total));
            }
        }

        // SAFETY: `src_io_ptr` is valid for `chunk` bytes and we advance by `chunk`.
        src_io_ptr = unsafe { src_io_ptr.add(chunk) };
        count -= chunk;
    }

    Ok(())
}

/// Copies from regular memory to I/O memory with fallible source reads.
///
/// # Safety
///
/// - `dst_io_ptr` must be valid for MMIO writes of `count` bytes.
pub(crate) unsafe fn copy_from_reader_to_io(
    reader: &mut VmReader,
    mut dst_io_ptr: *mut u8,
    mut count: usize,
) -> Result<(), (Error, usize)> {
    if let Some(result) = unsafe { try_copy_reader_to_io_direct(reader, dst_io_ptr, count) } {
        return result;
    }

    // Fallback: direct user->MMIO copy unavailable; use bounce buffer.
    const BUF_SIZE: usize = 4096;
    let mut buf = [0u8; BUF_SIZE];
    let mut copied_total = 0;

    while count > 0 {
        let chunk = core::cmp::min(count, core::cmp::min(buf.len(), reader.remain()));

        if chunk == 0 {
            return Err((Error::PageFault, copied_total));
        }

        let mut writer = VmWriter::from(&mut buf[..chunk]);
        match reader.read_fallible(&mut writer) {
            Ok(read_len) => {
                if read_len < chunk {
                    // SAFETY: The MMIO range is valid for `read_len` bytes.
                    unsafe { copy_to_io_mem(buf.as_ptr(), dst_io_ptr, read_len) };
                    copied_total += read_len;
                    return Err((Error::PageFault, copied_total));
                }
            }
            Err((err, read_len)) => {
                if read_len > 0 {
                    // SAFETY: The MMIO range is valid for `read_len` bytes.
                    unsafe { copy_to_io_mem(buf.as_ptr(), dst_io_ptr, read_len) };
                    copied_total += read_len;
                }
                return Err((err, copied_total));
            }
        }

        // SAFETY: The MMIO range is valid for `chunk` bytes.
        unsafe { copy_to_io_mem(buf.as_ptr(), dst_io_ptr, chunk) };

        // SAFETY: `dst_io_ptr` is valid for `chunk` bytes and we advance by `chunk`.
        dst_io_ptr = unsafe { dst_io_ptr.add(chunk) };
        count -= chunk;
        copied_total += chunk;
    }

    Ok(())
}

/// Attempts a direct fallible copy from MMIO into a `VmWriter` without a bounce buffer.
///
/// This is only used when the bulk MMIO copy path is available at runtime.
unsafe fn try_copy_io_to_writer_direct(
    writer: &mut VmWriter,
    src_io_ptr: *const u8,
    count: usize,
) -> Option<Result<(), (Error, usize)>> {
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    if !tdx_guest::tdx_is_enabled() {
        return None;
    }

    if src_io_ptr.addr() & 7 != 0 {
        return None;
    }

    if count == 0 {
        return Some(Ok(()));
    }

    let copy_len = count.min(writer.avail());
    if copy_len == 0 {
        return Some(Err((Error::PageFault, 0)));
    }

    // SAFETY: The caller guarantees the MMIO range is valid for `copy_len` bytes and the
    // writer cursor is valid or in user space for `copy_len` bytes.
    let copied = unsafe { memcpy_fallible(writer.cursor(), src_io_ptr, copy_len) };
    writer.skip(copied);

    Some(if copied < count {
        Err((Error::PageFault, copied))
    } else {
        Ok(())
    })
}

/// Attempts a direct fallible copy to MMIO from a `VmReader` without a bounce buffer.
///
/// This is only used when the bulk MMIO copy path is available at runtime.
unsafe fn try_copy_reader_to_io_direct(
    reader: &mut VmReader,
    dst_io_ptr: *mut u8,
    count: usize,
) -> Option<Result<(), (Error, usize)>> {
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    if !tdx_guest::tdx_is_enabled() {
        return None;
    }

    if dst_io_ptr.addr() & 7 != 0 {
        return None;
    }

    if count == 0 {
        return Some(Ok(()));
    }

    let copy_len = count.min(reader.remain());
    if copy_len == 0 {
        return Some(Err((Error::PageFault, 0)));
    }

    // SAFETY: The caller guarantees the MMIO range is valid for `copy_len` bytes and the
    // reader cursor is valid or in user space for `copy_len` bytes.
    let copied = unsafe { memcpy_fallible(dst_io_ptr, reader.cursor(), copy_len) };
    reader.skip(copied);

    Some(if copied < count {
        Err((Error::PageFault, copied))
    } else {
        Ok(())
    })
}

/// CPU architecture-agnostic tests for `arch::io::io_mem::{read_once, write_once}`.
#[cfg(ktest)]
mod test_read_once_and_write_once {
    use crate::{
        arch::io::io_mem::{read_once, write_once},
        prelude::ktest,
    };

    #[ktest]
    fn read_write_u8() {
        let mut data: u8 = 0;
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data, 42u8);
            assert_eq!(read_once(&data), 42u8);
        }
    }

    #[ktest]
    fn read_write_u16() {
        let mut data: u16 = 0;
        let val: u16 = 0x1234;
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn read_write_u32() {
        let mut data: u32 = 0;
        let val: u32 = 0x12345678;
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn read_write_u64() {
        let mut data: u64 = 0;
        let val: u64 = 0xDEADBEEFCAFEBABE;
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn boundary_overlap() {
        // Ensure that writing a u8 doesn't corrupt neighboring bytes
        // in a larger structure, verifying our instruction sizing.
        let mut data: [u8; 2] = [0xAA, 0xBB];
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data[0], 0x11u8);
            assert_eq!(data[0], 0x11);
            assert_eq!(data[1], 0xBB); // Should remain untouched
        }
    }
}

#[cfg(ktest)]
mod test_copy_helpers {
    use super::{copy_from_io_mem, copy_to_io_mem};
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

        // SAFETY: The test buffers are valid for the requested range.
        unsafe { copy_from_io_mem(dst_ptr, src_ptr, len) };

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

        // SAFETY: The test buffers are valid for the requested range.
        unsafe { copy_to_io_mem(src_ptr, dst_ptr, len) };

        assert_eq!(
            &dst[dst_offset..dst_offset + len],
            &src[src_offset..src_offset + len]
        );
    }

    #[ktest]
    fn copy_from_alignment_and_sizes() {
        let word_size = size_of::<usize>();
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
        let word_size = size_of::<usize>();
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
