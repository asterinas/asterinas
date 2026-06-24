// SPDX-License-Identifier: MPL-2.0

use ostd::{
    Error as OstdError,
    mm::{Infallible, MAX_USERSPACE_VADDR, VmSpace},
};

use crate::{
    prelude::*,
    vm::{
        perms::VmPerms,
        vmar::{PageFaultInfo, Vmar},
    },
};

/// A kernel space I/O vector.
#[derive(Clone, Copy, Debug)]
struct IoVec {
    base: Vaddr,
    len: usize,
}

/// A user space I/O vector.
///
/// The difference between `IoVec` and `UserIoVec`
/// is that `UserIoVec` uses `isize` as the length type,
/// while `IoVec` uses `usize`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct UserIoVec {
    base: Vaddr,
    len: isize,
}

impl TryFrom<UserIoVec> for IoVec {
    type Error = Error;

    fn try_from(value: UserIoVec) -> Result<Self> {
        if value.len < 0 {
            return_errno_with_message!(Errno::EINVAL, "the I/O buffer length cannot be negative");
        }

        Ok(IoVec {
            base: value.base,
            len: value.len as usize,
        })
    }
}

impl IoVec {
    /// Returns whether the `IoVec` points to an empty user buffer.
    const fn is_empty(&self) -> bool {
        self.len == 0 || self.base == 0
    }

    fn reader<'a>(&self, vm_space: &'a VmSpace) -> Result<VmReader<'a>> {
        Ok(vm_space.reader(self.base, self.len)?)
    }

    fn writer<'a>(&self, vm_space: &'a VmSpace) -> Result<VmWriter<'a>> {
        Ok(vm_space.writer(self.base, self.len)?)
    }
}

/// The maximum number of buffers in the I/O vector.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.16/source/include/uapi/linux/uio.h#L46>.
pub(super) const MAX_IO_VECTOR_LENGTH: usize = 1024;
/// The maximum bytes of all buffers in the I/O vector.
///
/// According to man pages, the kernel should fail with [`Errno::EINVAL`] if the number of bytes in
/// the I/O vector exceeds this threshold. See
/// <https://man7.org/linux/man-pages/man2/writev.2.html>.
///
/// However, the actual Linux behavior is to truncate the buffer and ignore the remaining buffer
/// space. See <https://elixir.bootlin.com/linux/v6.12.6/source/lib/iov_iter.c#L1463>.
///
/// Typical 64-bit architectures do not have 64-bit virtual address space, and the value of
/// [`MAX_IO_VECTOR_LENGTH`] is relatively small. Therefore, userspace may not be able to supply a
/// valid I/O vector containing so many bytes. Nevertheless, we should still check against this to
/// prevent overflows in the future, e.g., when the virtual address space becomes larger.
const MAX_TOTAL_IOV_BYTES: usize = isize::MAX as usize;

/// The util function for create [`VmReader`]/[`VmWriter`]s.
fn copy_iovs_and_convert<'a, T: 'a>(
    user_space: &'a CurrentUserSpace<'a>,
    start_addr: Vaddr,
    count: usize,
    convert_iovec: impl Fn(&IoVec, &'a VmSpace) -> Result<T>,
) -> Result<(Box<[IoVec]>, Box<[T]>)> {
    if count > MAX_IO_VECTOR_LENGTH {
        return_errno_with_message!(Errno::EINVAL, "the I/O vector contains too many buffers");
    }

    let vm_space = user_space.vmar().vm_space();

    let mut iovs = Vec::with_capacity(count);
    let mut v = Vec::with_capacity(count);
    let mut max_len = MAX_TOTAL_IOV_BYTES;

    for idx in 0..count {
        let mut iov = {
            let addr = start_addr + idx * size_of::<UserIoVec>();
            let uiov: UserIoVec = vm_space.reader(addr, size_of::<UserIoVec>())?.read_val()?;
            IoVec::try_from(uiov)?
        };

        // Truncate the buffer if the number of bytes exceeds `MAX_TOTAL_IOV_BYTES`.
        // See comments above the `MAX_TOTAL_IOV_BYTES` constant for more details.
        if iov.len > max_len {
            iov.len = max_len;
        }
        max_len -= iov.len;

        if iov.is_empty() {
            continue;
        }

        let converted = convert_iovec(&iov, vm_space)?;
        iovs.push(iov);
        v.push(converted)
    }

    Ok((iovs.into_boxed_slice(), v.into_boxed_slice()))
}

/// A collection of [`VmReader`]s.
///
/// Such readers are built from user-provided buffer, so it's always fallible.
pub struct VmReaderArray<'a> {
    iovs: Box<[IoVec]>,
    readers: Box<[VmReader<'a>]>,
    vmar: Option<&'a Vmar>,
}

/// A collection of [`VmWriter`]s.
///
/// Such writers are built from user-provided buffer, so it's always fallible.
pub struct VmWriterArray<'a> {
    iovs: Box<[IoVec]>,
    writers: Box<[VmWriter<'a>]>,
    vmar: Option<&'a Vmar>,
}

fn prefault_user_range(vmar: &Vmar, base: Vaddr, len: usize, perms: VmPerms) -> Result<()> {
    if len == 0 {
        return Ok(());
    }

    let last = base
        .checked_add(len - 1)
        .ok_or_else(|| Error::with_message(Errno::EFAULT, "the I/O vector range overflows"))?;
    let mut page = base / PAGE_SIZE * PAGE_SIZE;
    let last_page = last / PAGE_SIZE * PAGE_SIZE;

    loop {
        vmar.handle_page_fault(&PageFaultInfo::new(page, perms))?;
        if page == last_page {
            return Ok(());
        }
        page += PAGE_SIZE;
    }
}

fn prefault_current_user_range(base: Vaddr, len: usize, perms: VmPerms) -> Result<()> {
    if len == 0 || base >= MAX_USERSPACE_VADDR {
        return Ok(());
    }

    let end = base
        .checked_add(len)
        .ok_or_else(|| Error::with_message(Errno::EFAULT, "the user buffer range overflows"))?;
    if end > MAX_USERSPACE_VADDR {
        return_errno_with_message!(Errno::EFAULT, "the user buffer range exceeds user space");
    }

    let task = ostd::task::Task::current()
        .ok_or_else(|| Error::with_message(Errno::EFAULT, "there is no current task"))?;
    let thread_local = crate::process::posix_thread::AsThreadLocal::as_thread_local(&task)
        .ok_or_else(|| Error::with_message(Errno::EFAULT, "there is no current user space"))?;
    let user_space = crate::context::CurrentUserSpace::new(thread_local);
    prefault_user_range(user_space.vmar(), base, len, perms)
}

fn prefault_iovs(
    vmar: Option<&Vmar>,
    iovs: &[IoVec],
    mut nbytes: usize,
    perms: VmPerms,
) -> Result<()> {
    let Some(vmar) = vmar else {
        return Ok(());
    };

    for iov in iovs {
        if nbytes == 0 {
            break;
        }

        let len = iov.len.min(nbytes);
        prefault_user_range(vmar, iov.base, len, perms)?;
        nbytes -= len;
    }

    Ok(())
}

fn advance_iovs(iovs: &mut [IoVec], mut nbytes: usize) {
    for iov in iovs {
        let skipped = iov.len.min(nbytes);
        iov.base += skipped;
        iov.len -= skipped;
        nbytes -= skipped;

        if nbytes == 0 {
            return;
        }
    }
}

impl<'a> VmReaderArray<'a> {
    /// Creates a new `VmReaderArray` from user-provided I/O vector buffers.
    ///
    /// This ensures that empty buffers are filtered out, meaning that all of the returned readers
    /// should be non-empty.
    pub fn from_user_io_vecs(
        user_space: &'a CurrentUserSpace<'a>,
        start_addr: Vaddr,
        count: usize,
    ) -> Result<Self> {
        let (iovs, readers) = copy_iovs_and_convert(user_space, start_addr, count, IoVec::reader)?;
        Ok(Self {
            iovs,
            readers,
            vmar: Some(user_space.vmar()),
        })
    }

    /// Returns mutable reference to [`VmReader`]s.
    pub fn readers_mut(&mut self) -> &mut [VmReader<'a>] {
        &mut self.readers
    }

    /// Creates a new `VmReaderArray`.
    #[cfg(ktest)]
    pub const fn new(readers: Box<[VmReader<'a>]>) -> Self {
        Self {
            iovs: Box::new([]),
            readers,
            vmar: None,
        }
    }
}

impl<'a> VmWriterArray<'a> {
    /// Creates a new `VmWriterArray` from user-provided I/O vector buffers.
    ///
    /// This ensures that empty buffers are filtered out, meaning that all of the returned writers
    /// should be non-empty.
    pub fn from_user_io_vecs(
        user_space: &'a CurrentUserSpace<'a>,
        start_addr: Vaddr,
        count: usize,
    ) -> Result<Self> {
        let (iovs, writers) = copy_iovs_and_convert(user_space, start_addr, count, IoVec::writer)?;
        Ok(Self {
            iovs,
            writers,
            vmar: Some(user_space.vmar()),
        })
    }

    /// Returns mutable reference to [`VmWriter`]s.
    pub fn writers_mut(&mut self) -> &mut [VmWriter<'a>] {
        &mut self.writers
    }
}

/// Trait defining the read behavior for a collection of [`VmReader`]s.
pub trait MultiRead: ReadCString {
    /// Reads the exact number of bytes required to exhaust `self` or fill `writer`,
    /// accumulating total bytes read.
    ///
    /// If the return value is `Ok(n)`,
    /// then `n` should be `min(self.sum_lens(), writer.avail())`.
    ///
    /// # Errors
    ///
    /// This method returns [`OstdError::PageFault`] if a page fault occurs, along with
    /// the number of bytes copied before the error occurs. When an error is returned,
    /// both `self` and `writer` are advanced by the returned byte count.
    fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> Result<usize, (OstdError, usize)>;

    /// Ensures that up to `nbytes` of readable memory can be accessed without
    /// taking a page fault in the subsequent copy.
    fn prefault_read(&self, _nbytes: usize) -> Result<()> {
        Ok(())
    }

    /// Calculates the total length of data remaining to read.
    fn sum_lens(&self) -> usize;

    /// Checks if the data remaining to read is empty.
    fn is_empty(&self) -> bool {
        self.sum_lens() == 0
    }

    /// Skips the first `nbytes` bytes of data, or skips to the end if the readers have
    /// insufficient bytes.
    fn skip_some(&mut self, nbytes: usize);
}

/// Trait defining the write behavior for a collection of [`VmWriter`]s.
pub trait MultiWrite {
    /// Writes the exact number of bytes required to exhaust `writer` or fill `self`,
    /// accumulating total bytes read.
    ///
    /// If the return value is `Ok(n)`,
    /// then `n` should be `min(self.sum_lens(), reader.remain())`.
    ///
    /// # Errors
    ///
    /// This method returns [`OstdError::PageFault`] if a page fault occurs, along with
    /// the number of bytes copied before the error occurs. When an error is returned,
    /// both `self` and `reader` are advanced by the returned byte count.
    fn write(&mut self, reader: &mut VmReader<'_, Infallible>)
    -> Result<usize, (OstdError, usize)>;

    /// Ensures that up to `nbytes` of writable memory can be accessed without
    /// taking a page fault in the subsequent copy.
    fn prefault_write(&self, _nbytes: usize) -> Result<()> {
        Ok(())
    }

    /// Calculates the length of space available to write.
    fn sum_lens(&self) -> usize;

    /// Checks if the space available to write is empty.
    fn is_empty(&self) -> bool {
        self.sum_lens() == 0
    }

    /// Skips the first `nbytes` bytes of data, or skips to the end if the writers have
    /// insufficient bytes.
    fn skip_some(&mut self, nbytes: usize);
}

impl MultiRead for VmReaderArray<'_> {
    fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> Result<usize, (OstdError, usize)> {
        let mut total_len = 0;

        for reader in &mut self.readers {
            let copied_len = reader
                .read_fallible(writer)
                .map_err(|(err, copied_len)| (err, total_len + copied_len))?;
            total_len += copied_len;
            if !writer.has_avail() {
                break;
            }
        }
        advance_iovs(&mut self.iovs, total_len);
        Ok(total_len)
    }

    fn sum_lens(&self) -> usize {
        self.readers
            .iter()
            .map(|vm_reader| vm_reader.remain())
            .sum()
    }

    fn skip_some(&mut self, mut nbytes: usize) {
        advance_iovs(&mut self.iovs, nbytes);
        for reader in &mut self.readers {
            let bytes_to_skip = reader.remain().min(nbytes);
            reader.skip(bytes_to_skip);
            nbytes -= bytes_to_skip;

            if nbytes == 0 {
                return;
            }
        }
    }

    fn prefault_read(&self, nbytes: usize) -> Result<()> {
        prefault_iovs(self.vmar, &self.iovs, nbytes, VmPerms::READ)
    }
}

impl MultiRead for VmReader<'_> {
    fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> Result<usize, (OstdError, usize)> {
        self.read_fallible(writer)
    }

    fn sum_lens(&self) -> usize {
        self.remain()
    }

    fn prefault_read(&self, nbytes: usize) -> Result<()> {
        prefault_current_user_range(
            self.cursor().addr(),
            self.remain().min(nbytes),
            VmPerms::READ,
        )
    }

    fn skip_some(&mut self, nbytes: usize) {
        self.skip(self.remain().min(nbytes));
    }
}

impl dyn MultiRead + '_ {
    /// Reads a `T` value, returning a `None` if the readers have insufficient bytes.
    pub fn read_val_opt<T: Pod>(&mut self) -> Result<Option<T>> {
        let mut val = T::new_zeroed();
        let nbytes = self
            .read(&mut VmWriter::from(val.as_mut_bytes()))
            .map_err(|(err, _)| Error::from(err))?;

        if nbytes == size_of::<T>() {
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }
}

impl MultiWrite for VmWriterArray<'_> {
    fn write(
        &mut self,
        reader: &mut VmReader<'_, Infallible>,
    ) -> Result<usize, (OstdError, usize)> {
        let mut total_len = 0;

        for writer in &mut self.writers {
            let copied_len = writer
                .write_fallible(reader)
                .map_err(|(err, copied_len)| (err, total_len + copied_len))?;
            total_len += copied_len;
            if !reader.has_remain() {
                break;
            }
        }
        advance_iovs(&mut self.iovs, total_len);
        Ok(total_len)
    }

    fn sum_lens(&self) -> usize {
        self.writers.iter().map(|vm_writer| vm_writer.avail()).sum()
    }

    fn skip_some(&mut self, mut nbytes: usize) {
        advance_iovs(&mut self.iovs, nbytes);
        for writer in &mut self.writers {
            let bytes_to_skip = writer.avail().min(nbytes);
            writer.skip(bytes_to_skip);
            nbytes -= bytes_to_skip;

            if nbytes == 0 {
                return;
            }
        }
    }

    fn prefault_write(&self, nbytes: usize) -> Result<()> {
        prefault_iovs(self.vmar, &self.iovs, nbytes, VmPerms::WRITE)
    }
}

impl MultiWrite for VmWriter<'_> {
    fn write(
        &mut self,
        reader: &mut VmReader<'_, Infallible>,
    ) -> Result<usize, (OstdError, usize)> {
        self.write_fallible(reader)
    }

    fn sum_lens(&self) -> usize {
        self.avail()
    }

    fn prefault_write(&self, nbytes: usize) -> Result<()> {
        prefault_current_user_range(
            self.cursor().addr(),
            self.avail().min(nbytes),
            VmPerms::WRITE,
        )
    }

    fn skip_some(&mut self, nbytes: usize) {
        self.skip(self.avail().min(nbytes));
    }
}

impl dyn MultiWrite + '_ {
    /// Writes a `T` value, truncating the value if the writers have insufficient bytes.
    pub fn write_val_trunc<T: Pod>(&mut self, val: &T) -> Result<()> {
        let _nbytes = self
            .write(&mut VmReader::from(val.as_bytes()))
            .map_err(|(err, _)| Error::from(err))?;
        // `_nbytes` may be smaller than the value size. We ignore it to truncate the value.

        Ok(())
    }
}
