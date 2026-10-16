// SPDX-License-Identifier: MPL-2.0

//! NVRAM access provided by the selected RTC driver.
//!
//! Character-device frontends obtain a backend through [`backend`]. They handle
//! file offsets, user-memory copying and authorization outside its transactions.

mod error;

pub use error::Error;

/// Returns the NVRAM backend provided by the selected RTC driver.
///
/// Returns [`Error::Unavailable`] before RTC initialization or when the selected
/// driver does not provide NVRAM. Obtaining a backend does not authorize writes.
pub fn backend() -> Result<&'static dyn Backend, Error> {
    crate::RTC_DRIVER
        .get()
        .and_then(|rtc| rtc.nvram())
        .ok_or(Error::Unavailable)
}

/// Access to a platform's NVRAM region.
///
/// Offsets are relative to the region, excluding the RTC registers. Callers
/// must authorize writes and initialization, which can change firmware settings.
/// Buffers must be kernel slices; user-memory copying belongs to the frontend.
pub trait Backend: Send + Sync {
    /// Returns the capacity of the region in bytes.
    fn size(&self) -> usize;

    /// Reads bytes at the given offset.
    ///
    /// Returns a short read at the end of the region and zero at or beyond it.
    /// On PC CMOS, a checksum mismatch fails reads within the region, even when
    /// the output buffer is empty, without modifying the output buffer.
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, Error>;

    /// Writes bytes at the given offset and updates the checksum.
    ///
    /// Returns a short write at the end of the region and zero at or beyond it.
    /// On PC CMOS, the old checksum must be valid before writing. The backend
    /// maintains checksum bytes, which may differ from the supplied values.
    fn write(&self, offset: usize, buffer: &[u8]) -> Result<usize, Error>;

    /// Writes zero to the region and initializes its checksum.
    ///
    /// This destroys firmware settings. On PCs, it can also change the RTC date
    /// through century registers; device-managed bytes may read back nonzero.
    /// A Linux-compatible frontend uses this for `NVRAM_INIT`, the ioctl that
    /// initializes NVRAM contents and checksum, after checking `CAP_SYS_ADMIN`.
    fn initialize(&self);

    /// Recomputes the checksum while preserving every other byte.
    ///
    /// A Linux-compatible frontend uses this for `NVRAM_SETCKS`, the ioctl that
    /// repairs the checksum without clearing NVRAM, after checking `CAP_SYS_ADMIN`.
    fn set_checksum(&self);
}
