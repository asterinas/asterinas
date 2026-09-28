// SPDX-License-Identifier: MPL-2.0

//! NVRAM operations sharing the selected RTC backend.
//!
//! Offsets address the NVRAM region rather than the RTC registers. Callers must
//! authorize writes and initialization: these operations can change firmware settings.

use crate::RTC_DRIVER;

mod error;

pub use error::Error;

/// Returns the capacity of the NVRAM region in bytes.
pub fn size() -> Result<usize, Error> {
    Ok(backend()?.size())
}

/// Reads NVRAM bytes at the given offset.
///
/// Returns a short read at the end of the region and zero at or beyond its end.
/// On x86, a checksum mismatch fails reads starting within the region, even
/// when the output buffer is empty.
pub fn read(offset: usize, buffer: &mut [u8]) -> Result<usize, Error> {
    backend()?.read(offset, buffer)
}

/// Writes NVRAM bytes at the given offset and updates the checksum.
///
/// Returns a short write at the end of the region and zero at or beyond its end.
/// On x86, the old checksum must be valid before the write. Checksum bytes are
/// maintained by the backend and may differ from bytes supplied by the caller.
pub fn write(offset: usize, buffer: &[u8]) -> Result<usize, Error> {
    backend()?.write(offset, buffer)
}

/// Writes zero to the NVRAM region and initializes its checksum.
///
/// This destroys firmware settings stored in the region. Authorization belongs
/// to the caller; it must check the permission associated with `NVRAM_INIT`.
/// On PC platforms the region may also contain RTC century registers, so this
/// operation can change the RTC date. Device-managed bytes may subsequently
/// read back differently from zero.
pub fn initialize() -> Result<(), Error> {
    backend()?.initialize();
    Ok(())
}

/// Recomputes the checksum without changing the other NVRAM bytes.
///
/// Authorization belongs to the caller, as for `NVRAM_SETCKS`.
pub fn set_checksum() -> Result<(), Error> {
    backend()?.set_checksum();
    Ok(())
}

fn backend() -> Result<&'static dyn Backend, Error> {
    RTC_DRIVER
        .get()
        .and_then(|rtc| rtc.nvram())
        .ok_or(Error::Unavailable)
}

pub(crate) trait Backend {
    fn size(&self) -> usize;
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, Error>;
    fn write(&self, offset: usize, buffer: &[u8]) -> Result<usize, Error>;
    fn initialize(&self);
    fn set_checksum(&self);
}
