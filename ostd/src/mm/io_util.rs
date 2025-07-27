// SPDX-License-Identifier: MPL-2.0

//! Utilities for types in [`super::io`].

use inherit_methods_macro::inherit_methods;
use ostd_pod::Pod;

use super::{Infallible, PodOnce, VmIo, VmIoFill, VmIoOnce, VmReader, VmWriter};
use crate::{
    mm::{FallibleVmRead, FallibleVmWrite},
    prelude::*,
    Error,
};

/// A helper trait that denotes types that can provide [`VmReader`]s and [`VmWriter`]s.
///
/// Having the reader and writer means that the type is capable of performing a range of VM
/// operations. Thus, several traits will be automatically and efficiently implemented, such as
/// [`VmIo`], [`VmIoFill`], and [`VmIoOnce`].
pub trait HasVmReaderWriter {
    /// A marker type that denotes the return types of [`Self::reader`] and [`Self::writer`].
    ///
    /// This can be either [`VmReaderWriterIdentity`] or [`VmReaderWriterResult`].
    //
    // TODO: This exists because `DmaStream` and related types track the DMA direction at runtime.
    // The goal is to achieve this at compile time, which would eliminate the need for
    // `VmReaderWriterTypes`. See the discussion at
    // <https://github.com/asterinas/asterinas/pull/2289#discussion_r2261801694>.
    type Types: VmReaderWriterTypes;

    /// Returns a reader to read data from it.
    fn reader(&self) -> <Self::Types as VmReaderWriterTypes>::Reader<'_>;
    /// Returns a writer to write data to it.
    fn writer(&self) -> <Self::Types as VmReaderWriterTypes>::Writer<'_>;
}

/// A marker trait that denotes the return types for [`HasVmReaderWriter`].
pub trait VmReaderWriterTypes {
    /// The return type of [`HasVmReaderWriter::reader`].
    type Reader<'a>;
    /// The return type of [`HasVmReaderWriter::writer`].
    type Writer<'a>;

    /// Converts [`Self::Reader`] to [`Result<VmReader<Infallible>>`].
    fn to_reader_result(reader: Self::Reader<'_>) -> Result<VmReader<'_, Infallible>>;
    /// Converts [`Self::Writer`] to [`Result<VmWriter<Infallible>>`].
    fn to_writer_result(writer: Self::Writer<'_>) -> Result<VmWriter<'_, Infallible>>;
}

/// A marker type that denotes reader and writer identities as [`HasVmReaderWriter`] return types.
pub enum VmReaderWriterIdentity {}
impl VmReaderWriterTypes for VmReaderWriterIdentity {
    type Reader<'a> = VmReader<'a, Infallible>;
    type Writer<'a> = VmWriter<'a, Infallible>;
    fn to_reader_result(reader: Self::Reader<'_>) -> Result<VmReader<'_, Infallible>> {
        Ok(reader)
    }
    fn to_writer_result(writer: Self::Writer<'_>) -> Result<VmWriter<'_, Infallible>> {
        Ok(writer)
    }
}

/// A marker type that denotes reader and writer results as [`HasVmReaderWriter`] return types.
pub enum VmReaderWriterResult {}
impl VmReaderWriterTypes for VmReaderWriterResult {
    type Reader<'a> = Result<VmReader<'a, Infallible>>;
    type Writer<'a> = Result<VmWriter<'a, Infallible>>;
    fn to_reader_result(reader: Self::Reader<'_>) -> Result<VmReader<'_, Infallible>> {
        reader
    }
    fn to_writer_result(writer: Self::Writer<'_>) -> Result<VmWriter<'_, Infallible>> {
        writer
    }
}

impl<S: HasVmReaderWriter + Send + Sync> VmIo for S {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let mut reader = <Self as HasVmReaderWriter>::Types::to_reader_result(self.reader())?;

        let limit = offset.checked_add(writer.avail()).ok_or(Error::Overflow)?;
        if limit > reader.remain() {
            return Err(Error::InvalidArgs);
        }

        reader.skip(offset);
        let _len = reader
            .to_fallible()
            .read_fallible(writer)
            .map_err(|(err, _)| err)?;
        debug_assert!(!writer.has_avail());
        Ok(())
    }

    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let mut reader = <Self as HasVmReaderWriter>::Types::to_reader_result(self.reader())?;

        let limit = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if limit > reader.remain() {
            return Err(Error::InvalidArgs);
        }

        let len = reader.skip(offset).read(&mut VmWriter::from(&mut *buf));
        debug_assert_eq!(len, buf.len());
        Ok(())
    }

    // No need to implement `read_slice`. Its default implementation is efficient enough by relying
    // on `read_bytes`.

    fn read_val<T: Pod>(&self, offset: usize) -> Result<T> {
        let mut reader = <Self as HasVmReaderWriter>::Types::to_reader_result(self.reader())?;

        if offset > reader.remain() {
            return Err(Error::InvalidArgs);
        }

        reader.skip(offset).read_val()
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let mut writer = <Self as HasVmReaderWriter>::Types::to_writer_result(self.writer())?;

        let limit = offset.checked_add(reader.remain()).ok_or(Error::Overflow)?;
        if limit > writer.avail() {
            return Err(Error::InvalidArgs);
        }

        writer.skip(offset);
        let _len = writer
            .to_fallible()
            .write_fallible(reader)
            .map_err(|(err, _)| err)?;
        debug_assert!(!reader.has_remain());
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let mut writer = <Self as HasVmReaderWriter>::Types::to_writer_result(self.writer())?;

        let limit = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if limit > writer.avail() {
            return Err(Error::InvalidArgs);
        }

        let len = writer.skip(offset).write(&mut VmReader::from(buf));
        debug_assert_eq!(len, buf.len());
        Ok(())
    }

    // No need to implement `write_slice`. Its default implementation is efficient enough by
    // relying on `write_bytes`.

    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> Result<()> {
        let mut writer = <Self as HasVmReaderWriter>::Types::to_writer_result(self.writer())?;

        if offset > writer.avail() {
            return Err(Error::InvalidArgs);
        }

        writer.skip(offset).write_val(new_val)
    }
}

impl<S: HasVmReaderWriter> VmIoFill for S {
    fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)> {
        let mut writer = <Self as HasVmReaderWriter>::Types::to_writer_result(self.writer())
            .map_err(|err| (err, 0))?;

        if offset > writer.avail() {
            return Err((Error::InvalidArgs, 0));
        }

        let filled_len = writer.skip(offset).fill_zeros(len);
        if filled_len == len {
            Ok(())
        } else {
            Err((Error::InvalidArgs, filled_len))
        }
    }
}

impl<S: HasVmReaderWriter> VmIoOnce for S {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T> {
        let mut reader = <Self as HasVmReaderWriter>::Types::to_reader_result(self.reader())?;
        reader.skip(offset).read_once()
    }

    fn write_once<T: PodOnce>(&self, offset: usize, new_val: &T) -> Result<()> {
        let mut writer = <Self as HasVmReaderWriter>::Types::to_writer_result(self.writer())?;
        writer.skip(offset).write_once(new_val)
    }
}

// The pointer implementations below (i.e., `impl_vm_io_pointer`/`impl_vm_io_once_pointer`) should
// apply to the `VmIo`/`VmIoOnce` traits themselves, instead of these helper traits.
//
// However, there are some unexpected compiler errors that complain that downstream crates can
// implement `HasVmReaderWriter` to cause conflict implementations.

macro_rules! impl_vm_io_pointer {
    ($typ:ty,$from:tt) => {
        #[inherit_methods(from = $from)]
        impl<T: HasVmReaderWriter> HasVmReaderWriter for $typ {
            type Types = T::Types;
            fn reader(&self) -> <Self::Types as VmReaderWriterTypes>::Reader<'_>;
            fn writer(&self) -> <Self::Types as VmReaderWriterTypes>::Writer<'_>;
        }
    };
}

impl_vm_io_pointer!(&T, "(**self)");
impl_vm_io_pointer!(&mut T, "(**self)");
impl_vm_io_pointer!(Box<T>, "(**self)");
impl_vm_io_pointer!(Arc<T>, "(**self)");
