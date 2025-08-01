// SPDX-License-Identifier: MPL-2.0

use inherit_methods_macro::inherit_methods;
use ostd_pod::Pod;

use super::{Infallible, PodOnce, VmIo, VmIoOnce, VmReader, VmWriter};
use crate::{
    mm::{FallibleVmRead, FallibleVmWrite},
    prelude::*,
    Error,
};

/// A helper trait that provides efficient [`VmIo`] implementation for types that can provide
/// [`VmReader`]s and [`VmWriter`]s.
pub(crate) trait VmIoByReaderWriter: Send + Sync {
    fn io_reader(&self) -> Result<VmReader<Infallible>>;
    fn io_writer(&self) -> Result<VmWriter<Infallible>>;
}

impl<S: VmIoByReaderWriter> VmIo for S {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let mut reader = self.io_reader()?;

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
        let mut reader = self.io_reader()?;

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
        let mut reader = self.io_reader()?;

        if offset > reader.remain() {
            return Err(Error::InvalidArgs);
        }

        reader.skip(offset).read_val()
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let mut writer = self.io_writer()?;

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
        let mut writer = self.io_writer()?;

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
        let mut writer = self.io_writer()?;

        if offset > writer.avail() {
            return Err(Error::InvalidArgs);
        }

        writer.skip(offset).write_val(new_val)
    }

    fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)> {
        let mut writer = self.io_writer().map_err(|err| (err, 0))?;

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

/// A helper trait that provides [`VmIoOnce`] implementation for types that can provide
/// [`VmReader`]s and [`VmWriter`]s.
pub(crate) trait VmIoOnceByReaderWriter: VmIoByReaderWriter {}

impl<S: VmIoOnceByReaderWriter> VmIoOnce for S {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T> {
        self.io_reader()?.skip(offset).read_once()
    }

    fn write_once<T: PodOnce>(&self, offset: usize, new_val: &T) -> Result<()> {
        self.io_writer()?.skip(offset).write_once(new_val)
    }
}

// The pointer implementations below (i.e., `impl_vm_io_pointer`/`impl_vm_io_once_pointer`) should
// apply to the `VmIo`/`VmIoOnce` traits themselves, instead of these helper traits.
//
// However, there are some unexpected compiler errors that complain that downstream crates can
// implement `VmIoByReaderWriter`/`VmIoOnceByReaderWriter` to cause conflict implementations --
// which isn't really possible because `VmIoByReaderWriter`/`VmIoOnceByReaderWriter` isn't supposed
// to be public.

macro_rules! impl_vm_io_pointer {
    ($typ:ty,$from:tt) => {
        #[inherit_methods(from = $from)]
        impl<T: VmIoByReaderWriter> VmIoByReaderWriter for $typ {
            fn io_reader(&self) -> Result<VmReader<Infallible>>;
            fn io_writer(&self) -> Result<VmWriter<Infallible>>;
        }
    };
}

impl_vm_io_pointer!(&T, "(**self)");
impl_vm_io_pointer!(&mut T, "(**self)");
impl_vm_io_pointer!(Box<T>, "(**self)");
impl_vm_io_pointer!(Arc<T>, "(**self)");

macro_rules! impl_vm_io_once_pointer {
    ($typ:ty,$from:tt) => {
        #[inherit_methods(from = $from)]
        impl<T: VmIoOnceByReaderWriter> VmIoOnceByReaderWriter for $typ {}
    };
}

impl_vm_io_once_pointer!(&T, "(**self)");
impl_vm_io_once_pointer!(&mut T, "(**self)");
impl_vm_io_once_pointer!(Box<T>, "(**self)");
impl_vm_io_once_pointer!(Arc<T>, "(**self)");
