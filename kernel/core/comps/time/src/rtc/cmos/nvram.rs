// SPDX-License-Identifier: MPL-2.0

//! PC CMOS NVRAM layout and checksum transactions.
//!
//! Follows the x86 behavior in Linux `drivers/char/nvram.c`. All operations
//! run with the shared CMOS lock held. Buffers are kernel slices, so no user
//! memory access, allocation, or sleeping occurs inside the critical section.

use crate::NvramError;

pub(super) const FIRST_BYTE: u8 = 14;
pub(super) const SIZE: usize = 128 - FIRST_BYTE as usize;
const CHECKSUM_START: u8 = 2;
const CHECKSUM_END: u8 = 31;
const CHECKSUM_HIGH: u8 = 32;
const CHECKSUM_LOW: u8 = 33;

pub(super) trait Access {
    fn read_byte(&mut self, offset: u8) -> u8;
    fn write_byte(&mut self, offset: u8, value: u8);
}

impl dyn Access + '_ {
    pub(super) fn read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<usize, NvramError> {
        if offset >= SIZE {
            return Ok(0);
        }
        self.check_checksum()?;
        let len = buffer.len().min(SIZE - offset);
        for (index, byte) in buffer[..len].iter_mut().enumerate() {
            *byte = self.read_byte((offset + index) as u8);
        }
        Ok(len)
    }

    pub(super) fn write(&mut self, offset: usize, buffer: &[u8]) -> Result<usize, NvramError> {
        if offset >= SIZE {
            return Ok(0);
        }
        self.check_checksum()?;
        let len = buffer.len().min(SIZE - offset);
        for (index, byte) in buffer[..len].iter().enumerate() {
            self.write_byte((offset + index) as u8, *byte);
        }
        self.set_checksum();
        Ok(len)
    }

    pub(super) fn initialize(&mut self) {
        for offset in 0..SIZE {
            self.write_byte(offset as u8, 0);
        }
        self.set_checksum();
    }

    pub(super) fn set_checksum(&mut self) {
        let [high, low] = self.checksum().to_be_bytes();
        self.write_byte(CHECKSUM_HIGH, high);
        self.write_byte(CHECKSUM_LOW, low);
    }

    fn check_checksum(&mut self) -> Result<(), NvramError> {
        let actual = self.checksum();
        let expected =
            u16::from_be_bytes([self.read_byte(CHECKSUM_HIGH), self.read_byte(CHECKSUM_LOW)]);
        if actual != expected {
            return Err(NvramError::InvalidChecksum);
        }
        Ok(())
    }

    fn checksum(&mut self) -> u16 {
        // At most 30 * 255 = 7650, which fits in u16.
        (CHECKSUM_START..=CHECKSUM_END)
            .map(|offset| u16::from(self.read_byte(offset)))
            .sum()
    }
}

#[cfg(any(test, ktest))]
mod test {
    use super::*;

    struct Memory([u8; SIZE]);

    impl Access for Memory {
        fn read_byte(&mut self, offset: u8) -> u8 {
            self.0[usize::from(offset)]
        }

        fn write_byte(&mut self, offset: u8, value: u8) {
            self.0[usize::from(offset)] = value;
        }
    }

    #[cfg_attr(ktest, ostd::prelude::ktest)]
    #[cfg_attr(test, test)]
    fn roundtrip_and_big_endian_checksum() {
        let mut memory = Memory([0; SIZE]);
        let payload = [0xff; 30];
        assert_eq!((&mut memory as &mut dyn Access).write(2, &payload), Ok(30));
        // Thirty bytes of 255 sum to 7650 = 0x1de2.
        assert_eq!(&memory.0[32..34], &[0x1d, 0xe2]);
        let mut output = [0; 30];
        assert_eq!(
            (&mut memory as &mut dyn Access).read(2, &mut output),
            Ok(30)
        );
        assert_eq!(output, payload);
    }

    #[cfg_attr(ktest, ostd::prelude::ktest)]
    #[cfg_attr(test, test)]
    fn invalid_checksum_does_not_mutate_data_or_output() {
        let mut memory = Memory([0; SIZE]);
        memory.0[2] = 1;
        let before = memory.0;
        let mut output = [0xaa; 4];
        assert_eq!(
            (&mut memory as &mut dyn Access).read(0, &mut output),
            Err(NvramError::InvalidChecksum)
        );
        assert_eq!(output, [0xaa; 4]);
        assert_eq!(
            (&mut memory as &mut dyn Access).write(0, &[3; 4]),
            Err(NvramError::InvalidChecksum)
        );
        assert_eq!(memory.0, before);
    }

    #[cfg_attr(ktest, ostd::prelude::ktest)]
    #[cfg_attr(test, test)]
    fn short_io_preserves_the_unused_buffer_tail() {
        let mut memory = Memory([0; SIZE]);
        assert_eq!(
            (&mut memory as &mut dyn Access).write(SIZE - 2, &[4, 5, 6, 7]),
            Ok(2)
        );
        let mut output = [0xaa; 4];
        assert_eq!(
            (&mut memory as &mut dyn Access).read(SIZE - 2, &mut output),
            Ok(2)
        );
        assert_eq!(output, [4, 5, 0xaa, 0xaa]);
    }

    #[cfg_attr(ktest, ostd::prelude::ktest)]
    #[cfg_attr(test, test)]
    fn eof_precedes_checksum_validation_and_cannot_overflow() {
        let mut memory = Memory([0xff; SIZE]);
        let before = memory.0;
        for offset in [SIZE, SIZE + 1, usize::MAX] {
            let mut output = [0xaa; 4];
            assert_eq!(
                (&mut memory as &mut dyn Access).read(offset, &mut output),
                Ok(0)
            );
            assert_eq!(
                (&mut memory as &mut dyn Access).write(offset, &[1; 4]),
                Ok(0)
            );
            assert_eq!(output, [0xaa; 4]);
        }
        assert_eq!(memory.0, before);
    }

    #[cfg_attr(ktest, ostd::prelude::ktest)]
    #[cfg_attr(test, test)]
    fn checksum_bytes_are_recomputed_after_explicit_overwrite() {
        let mut memory = Memory([0; SIZE]);
        assert_eq!((&mut memory as &mut dyn Access).write(2, &[1, 2, 3]), Ok(3));
        assert_eq!(
            (&mut memory as &mut dyn Access).write(32, &[0xff, 0xff]),
            Ok(2)
        );
        assert_eq!(&memory.0[32..34], &[0, 6]);
        assert_eq!(
            (&mut memory as &mut dyn Access).read(0, &mut [0; SIZE]),
            Ok(SIZE)
        );
    }

    #[cfg_attr(ktest, ostd::prelude::ktest)]
    #[cfg_attr(test, test)]
    fn initialization_repairs_corruption_and_clears_the_region() {
        let mut memory = Memory([0xff; SIZE]);
        (&mut memory as &mut dyn Access).initialize();
        assert_eq!(memory.0, [0; SIZE]);
        assert_eq!(
            (&mut memory as &mut dyn Access).read(0, &mut [0; SIZE]),
            Ok(SIZE)
        );
    }

    #[cfg_attr(ktest, ostd::prelude::ktest)]
    #[cfg_attr(test, test)]
    fn checksum_repair_preserves_every_non_checksum_byte() {
        let mut memory = Memory(core::array::from_fn(|index| index as u8));
        let before = memory.0;
        (&mut memory as &mut dyn Access).set_checksum();
        // Sum of 2 through 31 is 495 = 0x01ef.
        assert_eq!(&memory.0[32..34], &[1, 0xef]);
        assert_eq!(&memory.0[..32], &before[..32]);
        assert_eq!(&memory.0[34..], &before[34..]);
        assert_eq!(
            (&mut memory as &mut dyn Access).read(0, &mut [0; SIZE]),
            Ok(SIZE)
        );
    }

    #[cfg_attr(ktest, ostd::prelude::ktest)]
    #[cfg_attr(test, test)]
    fn zero_length_io_still_validates_checksum_inside_region() {
        let mut memory = Memory([0; SIZE]);
        assert_eq!((&mut memory as &mut dyn Access).read(0, &mut []), Ok(0));
        assert_eq!((&mut memory as &mut dyn Access).write(0, &[]), Ok(0));
        memory.0[2] = 1;
        assert_eq!(
            (&mut memory as &mut dyn Access).read(0, &mut []),
            Err(NvramError::InvalidChecksum)
        );
        assert_eq!(
            (&mut memory as &mut dyn Access).write(0, &[]),
            Err(NvramError::InvalidChecksum)
        );
    }
}
