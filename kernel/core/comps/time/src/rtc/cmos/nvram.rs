// SPDX-License-Identifier: MPL-2.0

//! PC CMOS NVRAM layout and checksum transactions.
//!
//! Follows the PC behavior in Linux `drivers/char/nvram.c`:
//! <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/nvram.c>.
//! Transactions hold the shared CMOS lock and use kernel slices, so they do not
//! access user memory, allocate or sleep while local IRQs are disabled.

use super::{CmosAccess, RtcCmos};
use crate::{NvramBackend, NvramError};

pub(super) const FIRST_BYTE: u8 = 14;
pub(super) const SIZE: usize = 128 - FIRST_BYTE as usize;
const CHECKSUM_START: u8 = 2;
const CHECKSUM_END: u8 = 31;
const CHECKSUM_HIGH: u8 = 32;
const CHECKSUM_LOW: u8 = 33;

impl NvramBackend for RtcCmos {
    fn size(&self) -> usize {
        SIZE
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, NvramError> {
        if offset >= SIZE {
            return Ok(0);
        }
        let mut access = self.access.lock();
        access.check_nvram_checksum()?;
        let len = buffer.len().min(SIZE - offset);
        for (index, byte) in buffer[..len].iter_mut().enumerate() {
            *byte = access.read_nvram_byte((offset + index) as u8);
        }
        Ok(len)
    }

    fn write(&self, offset: usize, buffer: &[u8]) -> Result<usize, NvramError> {
        if offset >= SIZE {
            return Ok(0);
        }
        let mut access = self.access.lock();
        access.check_nvram_checksum()?;
        let len = buffer.len().min(SIZE - offset);
        for (index, byte) in buffer[..len].iter().enumerate() {
            access.write_nvram_byte((offset + index) as u8, *byte);
        }
        access.set_nvram_checksum();
        Ok(len)
    }

    fn initialize(&self) {
        let mut access = self.access.lock();
        for offset in 0..SIZE {
            access.write_nvram_byte(offset as u8, 0);
        }
        access.set_nvram_checksum();
    }

    fn set_checksum(&self) {
        self.access.lock().set_nvram_checksum();
    }
}

impl CmosAccess {
    pub(super) fn read_nvram_byte(&mut self, offset: u8) -> u8 {
        debug_assert!(usize::from(offset) < SIZE);
        self.read_register_impl(FIRST_BYTE + offset)
    }

    pub(super) fn write_nvram_byte(&mut self, offset: u8, value: u8) {
        debug_assert!(usize::from(offset) < SIZE);
        self.io_sel.write(FIRST_BYTE + offset);
        self.io_val.write(value);
    }

    fn check_nvram_checksum(&mut self) -> Result<(), NvramError> {
        let actual = self.nvram_checksum();
        let expected = u16::from_be_bytes([
            self.read_nvram_byte(CHECKSUM_HIGH),
            self.read_nvram_byte(CHECKSUM_LOW),
        ]);
        if actual != expected {
            return Err(NvramError::InvalidChecksum);
        }
        Ok(())
    }

    fn set_nvram_checksum(&mut self) {
        let [high, low] = self.nvram_checksum().to_be_bytes();
        self.write_nvram_byte(CHECKSUM_HIGH, high);
        self.write_nvram_byte(CHECKSUM_LOW, low);
    }

    fn nvram_checksum(&mut self) -> u16 {
        // At most 30 * 255 = 7650, which fits in u16.
        (CHECKSUM_START..=CHECKSUM_END)
            .map(|offset| u16::from(self.read_nvram_byte(offset)))
            .sum()
    }
}
