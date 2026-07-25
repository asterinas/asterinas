// SPDX-License-Identifier: MPL-2.0

//! Loads the exFAT up-case table and provides case-folded name comparison helpers.
//!
//! This module owns the validated exFAT up-case table used for case-insensitive name lookup.
//! It decodes the on-disk compressed table representation,
//! verifies checksum and size-related constraints,
//! and publishes the Unicode code-unit mapping used by directory scan logic.
//!
//! Its entry points load the table from the mounted filesystem,
//! decode or expand compressed runs,
//! and perform the case-folded comparisons needed by lookup and rename validation.
//! The data model is the exFAT up-case mapping table anchored by the dedicated special file.
//!
//! Recovery semantics are conservative.
//! Malformed table contents, checksum mismatches, or impossible encoded runs are rejected
//! before lookup can depend on them.
//! This module is limited to exFAT up-case semantics
//! and does not own broader Unicode normalization policy.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 7.2 and 7.2.5.

use super::{
    boot::BootRegion,
    fat::{ChainVisitControl, FatReader},
    invalid_on_disk_layout,
};
use crate::prelude::*;

pub(super) const UPCASE_TABLE_ENTRY_TYPE: u8 = 0x82;

#[derive(Clone)]
pub(super) struct UpcaseTable {
    mapping: Vec<u16>,
}

impl UpcaseTable {
    pub(super) const NAME_MAX: usize = 255;
    const TABLE_CODE_UNIT_COUNT: usize = u16::MAX as usize + 1;
    const UNCOMPRESSED_TABLE_BYTE_LEN: usize = Self::TABLE_CODE_UNIT_COUNT * 2;
    const MAX_ENCODED_TABLE_BYTE_LEN: usize = 4 * Self::TABLE_CODE_UNIT_COUNT;
    const MANDATORY_PREFIX_CODE_UNIT_COUNT: u8 = 128;

    pub(super) fn load(
        boot_region: &BootRegion,
        fat_reader: &mut FatReader<'_>,
        upcase_entry: [u8; 32],
    ) -> Result<Self> {
        if upcase_entry[0] != UPCASE_TABLE_ENTRY_TYPE {
            return Err(invalid_on_disk_layout());
        }

        let checksum = u32::from_le_bytes([
            upcase_entry[4],
            upcase_entry[5],
            upcase_entry[6],
            upcase_entry[7],
        ]);
        let first_cluster = u32::from_le_bytes([
            upcase_entry[20],
            upcase_entry[21],
            upcase_entry[22],
            upcase_entry[23],
        ]);
        let data_length = u64::from_le_bytes([
            upcase_entry[24],
            upcase_entry[25],
            upcase_entry[26],
            upcase_entry[27],
            upcase_entry[28],
            upcase_entry[29],
            upcase_entry[30],
            upcase_entry[31],
        ]);
        boot_region.validate_stream_data(first_cluster, data_length)?;
        let data_length = usize::try_from(data_length).map_err(|_| invalid_on_disk_layout())?;
        if data_length > Self::MAX_ENCODED_TABLE_BYTE_LEN {
            return Err(invalid_on_disk_layout());
        }
        let mut remaining = data_length;
        let mut table_bytes = Vec::with_capacity(data_length);
        fat_reader.walk_cluster_chain(first_cluster, |_, cluster_bytes| {
            let bytes_to_copy = remaining.min(cluster_bytes.len());
            table_bytes.extend_from_slice(&cluster_bytes[..bytes_to_copy]);
            remaining -= bytes_to_copy;
            if remaining == 0 {
                return Ok(ChainVisitControl::Stop);
            }
            Ok(ChainVisitControl::Continue)
        })?;
        if remaining != 0 {
            return Err(invalid_on_disk_layout());
        }
        if Self::stream_checksum(&table_bytes) != checksum {
            return Err(invalid_on_disk_layout());
        }
        Ok(Self {
            mapping: Self::decode_mapping(&table_bytes)?,
        })
    }

    fn stream_checksum(bytes: &[u8]) -> u32 {
        let mut checksum = 0u32;
        for byte in bytes {
            checksum = checksum.rotate_right(1).wrapping_add(u32::from(*byte));
        }
        checksum
    }

    fn decode_mapping(table_bytes: &[u8]) -> Result<Vec<u16>> {
        let mapping = if table_bytes.len() == Self::UNCOMPRESSED_TABLE_BYTE_LEN {
            let mut mapping = Vec::with_capacity(Self::TABLE_CODE_UNIT_COUNT);
            for word in table_bytes.as_chunks::<2>().0 {
                mapping.push(u16::from_le_bytes([word[0], word[1]]));
            }
            mapping
        } else {
            let (words, remainder) = table_bytes.as_chunks::<2>();
            if !remainder.is_empty() {
                return Err(invalid_on_disk_layout());
            }

            let mut mapping = Vec::with_capacity(Self::TABLE_CODE_UNIT_COUNT);
            let mut words = words.iter();
            while let Some(word) = words.next() {
                let value = u16::from_le_bytes([word[0], word[1]]);
                if value != u16::MAX {
                    if mapping.len() == Self::TABLE_CODE_UNIT_COUNT {
                        return Err(invalid_on_disk_layout());
                    }
                    mapping.push(value);
                    continue;
                }

                let Some(identity_count_word) = words.next() else {
                    if mapping.len() == usize::from(u16::MAX) {
                        mapping.push(u16::MAX);
                        break;
                    }
                    return Err(invalid_on_disk_layout());
                };
                let identity_count =
                    u16::from_le_bytes([identity_count_word[0], identity_count_word[1]]);
                if identity_count == 0 {
                    return Err(invalid_on_disk_layout());
                }

                let run_end = mapping
                    .len()
                    .checked_add(usize::from(identity_count))
                    .ok_or_else(invalid_on_disk_layout)?;
                if run_end > Self::TABLE_CODE_UNIT_COUNT {
                    return Err(invalid_on_disk_layout());
                }

                for code_unit in mapping.len()..run_end {
                    mapping.push(u16::try_from(code_unit).map_err(|_| invalid_on_disk_layout())?);
                }
            }

            mapping
        };

        if mapping.len() != Self::TABLE_CODE_UNIT_COUNT {
            return Err(invalid_on_disk_layout());
        }

        for code_unit in 0..Self::MANDATORY_PREFIX_CODE_UNIT_COUNT {
            let expected_mapping = match code_unit {
                b'a'..=b'z' => u16::from(code_unit - b'a' + b'A'),
                _ => u16::from(code_unit),
            };
            if mapping[usize::from(code_unit)] != expected_mapping {
                return Err(invalid_on_disk_layout());
            }
        }

        Ok(mapping)
    }

    pub(super) fn upcase_code_unit(&self, code_unit: u16) -> u16 {
        self.mapping
            .get(usize::from(code_unit))
            .copied()
            .unwrap_or(code_unit)
    }

    pub(super) fn names_equal(&self, left: &[u16], right: &[u16]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right.iter())
                .all(|(left_code_unit, right_code_unit)| {
                    self.upcase_code_unit(*left_code_unit)
                        == self.upcase_code_unit(*right_code_unit)
                })
    }

    pub(super) fn name_hash(&self, name: &[u16]) -> u16 {
        let mut hash = 0u16;
        for code_unit in name {
            for byte in self.upcase_code_unit(*code_unit).to_le_bytes() {
                hash = hash.rotate_right(1).wrapping_add(u16::from(byte));
            }
        }
        hash
    }
}
