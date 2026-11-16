// SPDX-License-Identifier: MPL-2.0

//! CRC32C validation of the fixed EROFS superblock coverage.
//!
//! The fields and bytes must come from the same read snapshot. Success means
//! only that the checksum matches or its feature is disabled; this module does
//! not read a device or construct a fully validated superblock.
//!
//! Format contract:
//! <https://github.com/torvalds/linux/blob/ffc253263a1375a65fa6c9f62a893e9767fbebfa/fs/erofs/super.c#L51>.
//! Software polynomial:
//! <https://github.com/erofs/erofs-utils/blob/f36cadb5c563995ab3aa8572a60ed6b721b9557d/include/erofs/internal.h#L602>.

use super::{super::io, super_block::SuperBlockFields};

const CRC32C_POLYNOMIAL: u32 = 0x82f6_3b78;
const CRC32C_INITIAL_STATE: u32 = u32::MAX;
const CHECKSUM_OFFSET_BYTES: usize = 4;
const CHECKSUM_SIZE_BYTES: usize = size_of::<u32>();
const CHECKSUM_END_BYTES: usize = CHECKSUM_OFFSET_BYTES + CHECKSUM_SIZE_BYTES;

/// A mismatch between the recorded and computed superblock checksums.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct ChecksumMismatch;

/// Verifies the checksum using fields parsed from the same coverage bytes.
pub(in super::super) fn verify_superblock_checksum(
    coverage: &[u8; io::SUPER_BLOCK_COVERAGE_SIZE_BYTES],
    fields: &SuperBlockFields,
) -> Result<(), ChecksumMismatch> {
    if !fields.has_superblock_checksum() {
        return Ok(());
    }

    // Substitute only the checksum field, without mutating or copying coverage.
    let crc = crc32c(CRC32C_INITIAL_STATE, &coverage[..CHECKSUM_OFFSET_BYTES]);
    let crc = crc32c(crc, &[0; CHECKSUM_SIZE_BYTES]);
    let crc = crc32c(crc, &coverage[CHECKSUM_END_BYTES..]);
    if crc == fields.recorded_checksum() {
        Ok(())
    } else {
        Err(ChecksumMismatch)
    }
}

/// Updates an uncomplemented reflected CRC32C state with the supplied bytes.
pub(in super::super) fn crc32c(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..u8::BITS {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ CRC32C_POLYNOMIAL
            };
        }
    }
    crc
}
