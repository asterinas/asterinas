// SPDX-License-Identifier: MPL-2.0

//! Validation of serialized AppArmor attachment DFAs.
//!
//! The table format is defined by Linux v6.12's
//! [match.h](https://github.com/torvalds/linux/blob/v6.12/security/apparmor/include/match.h),
//! with unpacking and validation in
//! [match.c](https://github.com/torvalds/linux/blob/v6.12/security/apparmor/match.c).
//! This subset accepts unflagged tables without an equivalence-class table,
//! with 16-bit transition/state tables.
//! It derives one literal attachment path and discards the serialized DFA.

use alloc::{string::String, vec::Vec};

use aster_core::prelude::*;

const MAGIC: u32 = 0x1b5e_783d;
const HEADER_SIZE_MIN: usize = 14;
const TABLE_HEADER_SIZE: usize = 12;

/// Maximum attachment length excluding the terminating NUL.
/// Matches `aster_core::fs::utils::PATH_MAX - 1` without exposing core internals.
const MAX_ATTACHMENT_PATH_BYTES: usize = 4095;

const ACCEPT: usize = 0;
const BASE: usize = 1;
const CHECK: usize = 2;
const DEFAULT: usize = 3;
const EC: usize = 4;
const META_ID: u16 = 6;
const ACCEPT2: usize = 6;
const NEXT: usize = 7;

const HEADER_DIFF_ENCODE: u16 = 1;
const HEADER_OOB_TRANS: u16 = 2;
const BASE_DIFF_ENCODE: u32 = 0x8000_0000;
const BASE_OOB_TRANS: u32 = 0x2000_0000;
const BASE_FLAGS: u32 = 0xff00_0000;

#[derive(Clone, Copy)]
struct TableView<'a> {
    width: u8,
    len: usize,
    data: &'a [u8],
}

impl TableView<'_> {
    fn get(&self, index: usize) -> Result<u32> {
        if index >= self.len {
            return Err(protocol_error());
        }

        let start = index
            .checked_mul(usize::from(self.width))
            .ok_or_else(protocol_error)?;
        let end = start
            .checked_add(usize::from(self.width))
            .ok_or_else(protocol_error)?;
        let bytes = self.data.get(start..end).ok_or_else(protocol_error)?;
        match self.width {
            1 => Ok(u32::from(bytes[0])),
            2 => Ok(u16::from_be_bytes(bytes.try_into().map_err(|_| protocol_error())?).into()),
            4 => Ok(u32::from_be_bytes(
                bytes.try_into().map_err(|_| protocol_error())?,
            )),
            _ => Err(protocol_error()),
        }
    }
}

pub(super) fn validate(encoded: &[u8]) -> Result<()> {
    let tables = parse_table_set(encoded)?;
    let (state_count, transition_count) = verify_table_shapes(&tables)?;
    verify_state_bounds(&tables, state_count, transition_count)
}

/// Proves the supported DFA language is exactly one absolute UTF-8 pathname.
///
/// In the uncompressed v5 encoding, a failed CHECK selects DEFAULT directly;
/// it does not follow a fallback chain.
/// State zero must be a rejecting sink.
/// Every visited state must have exactly one non-sink edge, except the sole
/// accepting terminal.
/// This deliberately rejects general regex languages and noncanonical dead-state encodings.
/// The path walk is bounded by the pathname byte limit plus one terminal state.
pub(super) fn literal_path(encoded: &[u8], xmatch_len: u32) -> Result<String> {
    validate(encoded)?;
    let tables = parse_table_set(encoded)?;
    let accept = required_table(&tables, ACCEPT)?;
    let accept2 = required_table(&tables, ACCEPT2)?;
    let base = required_table(&tables, BASE)?;
    let default = required_table(&tables, DEFAULT)?;
    let check = required_table(&tables, CHECK)?;
    let next = required_table(&tables, NEXT)?;
    if xmatch_len == 0 {
        return Err(protocol_error());
    }
    if xmatch_len as usize > MAX_ATTACHMENT_PATH_BYTES {
        return Err(unsupported_error());
    }
    for state in 0..accept.len {
        // Legacy xmatch uses dfa_user_allow: bit 0 is MAY_EXEC.
        // Audit, other permissions, and ACCEPT2 extensions are outside this exact-path ABI.
        // Reference: <https://github.com/torvalds/linux/blob/db2ddb87143519e20a95aa36c60b36107b736a58/security/apparmor/policy_compat.c>.
        if !matches!(accept.get(state)?, 0 | 1) || accept2.get(state)? != 0 {
            return Err(unsupported_error());
        }
    }
    // Uncompressed states use DEFAULT directly after a failed CHECK.
    // Reference: <https://github.com/torvalds/linux/blob/db2ddb87143519e20a95aa36c60b36107b736a58/security/apparmor/match.c>.
    let transition_fn = |state: usize, byte: u8| -> Result<usize> {
        let index = base.get(state)? as usize + usize::from(byte);
        Ok(if check.get(index)? as usize == state {
            next.get(index)?
        } else {
            default.get(state)?
        } as usize)
    };
    if accept.get(0)? != 0 {
        return Err(protocol_error());
    }
    for byte in 0..=255 {
        if transition_fn(0, byte)? != 0 {
            return Err(protocol_error());
        }
    }
    let mut path = Vec::new();
    path.try_reserve_exact(xmatch_len as usize)
        .map_err(|_| Error::new(Errno::ENOMEM))?;
    let mut state = 1;
    for _ in 0..accept.len.min(MAX_ATTACHMENT_PATH_BYTES + 1) {
        let mut edge = None;
        for byte in 0..=255 {
            let target = transition_fn(state, byte)?;
            if target != 0 {
                if byte == 0 || edge.is_some() {
                    return Err(unsupported_error());
                }
                edge = Some((byte, target));
            }
        }
        if accept.get(state)? == 1 {
            if edge.is_some() || path.first() != Some(&b'/') {
                return Err(unsupported_error());
            }
            if path.len() != xmatch_len as usize {
                return Err(protocol_error());
            }
            return String::from_utf8(path).map_err(|_| Error::new(Errno::EILSEQ));
        }
        let (byte, target) = edge.ok_or_else(unsupported_error)?;
        // A repeated state cannot terminate this deterministic single-edge walk.
        // Bound the walk independently of the untrusted declared match length.
        if path.len() == MAX_ATTACHMENT_PATH_BYTES {
            return Err(unsupported_error());
        }
        path.try_reserve(1).map_err(|_| Error::new(Errno::ENOMEM))?;
        path.push(byte);
        state = target;
    }
    Err(unsupported_error())
}

fn parse_table_set(encoded: &[u8]) -> Result<[Option<TableView<'_>>; 8]> {
    if encoded.len() < HEADER_SIZE_MIN || read_u32(encoded, 0)? != MAGIC {
        return Err(protocol_error());
    }

    let header_size = read_u32(encoded, 4)? as usize;
    let serialized_size = read_u32(encoded, 8)? as usize;
    if header_size < HEADER_SIZE_MIN
        || header_size & 7 != 0
        || header_size > encoded.len()
        || serialized_size != encoded.len()
    {
        return Err(protocol_error());
    }

    let flags = read_u16(encoded, 12)?;
    let known_flags = HEADER_DIFF_ENCODE | HEADER_OOB_TRANS;
    if flags & !known_flags != 0 {
        return Err(protocol_error());
    }
    if flags != 0 {
        return Err(unsupported_error());
    }

    let mut tables = [None; 8];
    let mut offset = header_size;
    while offset < encoded.len() {
        let header_end = offset
            .checked_add(TABLE_HEADER_SIZE)
            .ok_or_else(protocol_error)?;
        if header_end > encoded.len() {
            return Err(protocol_error());
        }

        let id = read_u16(encoded, offset)?;
        if !(1..=8).contains(&id) || id == META_ID {
            return Err(protocol_error());
        }
        let index = usize::from(id - 1);
        if tables[index].is_some() {
            return Err(protocol_error());
        }

        let width = read_u16(encoded, offset + 2)?;
        if !matches!(width, 1 | 2 | 4) {
            return Err(protocol_error());
        }

        let high_len = read_u32(encoded, offset + 4)?;
        let len = read_u32(encoded, offset + 8)? as usize;
        if high_len != 0 || len == 0 {
            return Err(protocol_error());
        }

        let data_len = len
            .checked_mul(usize::from(width))
            .ok_or_else(protocol_error)?;
        let data_end = header_end
            .checked_add(data_len)
            .ok_or_else(protocol_error)?;
        let table_end = data_end
            .checked_add(7)
            .map(|end| end & !7)
            .ok_or_else(protocol_error)?;
        if table_end > encoded.len() {
            return Err(protocol_error());
        }

        tables[index] = Some(TableView {
            width: width as u8,
            len,
            data: &encoded[header_end..data_end],
        });
        offset = table_end;
    }

    if offset != encoded.len() {
        return Err(protocol_error());
    }
    Ok(tables)
}

fn verify_table_shapes(tables: &[Option<TableView<'_>>; 8]) -> Result<(usize, usize)> {
    if tables[EC].is_some() {
        return Err(unsupported_error());
    }

    let accept = required_table(tables, ACCEPT)?;
    let accept2 = required_table(tables, ACCEPT2)?;
    let base = required_table(tables, BASE)?;
    let default = required_table(tables, DEFAULT)?;
    let next = required_table(tables, NEXT)?;
    let check = required_table(tables, CHECK)?;

    if accept.width != 4 || accept2.width != 4 || base.width != 4 {
        return Err(protocol_error());
    }
    for table in [default, next, check] {
        match table.width {
            2 => {}
            4 => return Err(unsupported_error()),
            _ => return Err(protocol_error()),
        }
    }

    let state_count = accept.len;
    let transition_count = next.len;
    if state_count < 2
        || accept2.len != state_count
        || base.len != state_count
        || default.len != state_count
        || check.len != transition_count
    {
        return Err(protocol_error());
    }

    Ok((state_count, transition_count))
}

fn verify_state_bounds(
    tables: &[Option<TableView<'_>>; 8],
    state_count: usize,
    transition_count: usize,
) -> Result<()> {
    let default = required_table(tables, DEFAULT)?;
    let base = required_table(tables, BASE)?;
    let next = required_table(tables, NEXT)?;
    let check = required_table(tables, CHECK)?;

    for index in 0..state_count {
        if default.get(index)? as usize >= state_count {
            return Err(protocol_error());
        }

        let base_value = base.get(index)?;
        let flags = base_value & BASE_FLAGS;
        let known_flags = BASE_DIFF_ENCODE | BASE_OOB_TRANS;
        if flags & !known_flags != 0 {
            return Err(protocol_error());
        }
        if flags != 0 {
            return Err(unsupported_error());
        }
        if base_value
            .checked_add(255)
            .is_none_or(|last| last as usize >= transition_count)
        {
            return Err(protocol_error());
        }
    }

    for index in 0..transition_count {
        if next.get(index)? as usize >= state_count || check.get(index)? as usize >= state_count {
            return Err(protocol_error());
        }
    }

    Ok(())
}

fn required_table<'a>(tables: &[Option<TableView<'a>>; 8], index: usize) -> Result<TableView<'a>> {
    tables[index].ok_or_else(protocol_error)
}

fn read_u16(encoded: &[u8], offset: usize) -> Result<u16> {
    let end = offset.checked_add(2).ok_or_else(protocol_error)?;
    Ok(u16::from_be_bytes(
        encoded
            .get(offset..end)
            .ok_or_else(protocol_error)?
            .try_into()
            .map_err(|_| protocol_error())?,
    ))
}

fn read_u32(encoded: &[u8], offset: usize) -> Result<u32> {
    let end = offset.checked_add(4).ok_or_else(protocol_error)?;
    Ok(u32::from_be_bytes(
        encoded
            .get(offset..end)
            .ok_or_else(protocol_error)?
            .try_into()
            .map_err(|_| protocol_error())?,
    ))
}

fn protocol_error() -> Error {
    Error::new(Errno::EPROTO)
}

fn unsupported_error() -> Error {
    Error::new(Errno::EOPNOTSUPP)
}

#[cfg(ktest)]
mod tests {
    use alloc::{vec, vec::Vec};

    use aster_core::prelude::{Errno, Result};
    use ostd::prelude::ktest;

    use super::validate;

    const MAGIC: u32 = 0x1b5e_783d;
    const HEADER_SIZE: usize = 16;
    const TABLE_HEADER_SIZE: usize = 12;

    const ACCEPT: u16 = 1;
    const BASE: u16 = 2;
    const CHECK: u16 = 3;
    const DEFAULT: u16 = 4;
    const EC: u16 = 5;
    const META: u16 = 6;
    const ACCEPT2: u16 = 7;
    const NEXT: u16 = 8;

    fn set_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn set_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn read_u16(bytes: &[u8], offset: usize) -> u16 {
        u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn push_table(output: &mut Vec<u8>, id: u16, width: u16, values: &[u32]) {
        output.extend_from_slice(&id.to_be_bytes());
        output.extend_from_slice(&width.to_be_bytes());
        output.extend_from_slice(&0u32.to_be_bytes());
        output.extend_from_slice(&(values.len() as u32).to_be_bytes());
        for value in values {
            match width {
                1 => output.push(*value as u8),
                2 => output.extend_from_slice(&(*value as u16).to_be_bytes()),
                4 => output.extend_from_slice(&value.to_be_bytes()),
                _ => {}
            }
        }
        while output.len() & 7 != 0 {
            output.push(0);
        }
    }

    fn minimal_dfa() -> Vec<u8> {
        let mut output = Vec::new();
        output.extend_from_slice(&MAGIC.to_be_bytes());
        output.extend_from_slice(&(HEADER_SIZE as u32).to_be_bytes());
        output.extend_from_slice(&0u32.to_be_bytes());
        output.extend_from_slice(&0u16.to_be_bytes());
        output.extend_from_slice(&[0; 2]);

        push_table(&mut output, ACCEPT, 4, &[0, 0]);
        push_table(&mut output, BASE, 4, &[0, 0]);
        push_table(&mut output, CHECK, 2, &vec![0; 256]);
        push_table(&mut output, DEFAULT, 2, &[0, 0]);
        push_table(&mut output, ACCEPT2, 4, &[0, 0]);
        push_table(&mut output, NEXT, 2, &vec![0; 256]);

        let serialized_size = output.len() as u32;
        set_u32(&mut output, 8, serialized_size);
        output
    }

    fn table_range(bytes: &[u8], wanted_id: u16) -> (usize, usize) {
        let mut offset = read_u32(bytes, 4) as usize;
        while offset < bytes.len() {
            let width = read_u16(bytes, offset + 2) as usize;
            let len = read_u32(bytes, offset + 8) as usize;
            let end = (offset + TABLE_HEADER_SIZE + width * len + 7) & !7;
            if read_u16(bytes, offset) == wanted_id {
                return (offset, end);
            }
            offset = end;
        }
        panic!("table not found")
    }

    fn replace_table(bytes: &mut Vec<u8>, id: u16, width: u16, values: &[u32]) {
        let (start, end) = table_range(bytes, id);
        let mut replacement = Vec::new();
        push_table(&mut replacement, id, width, values);
        bytes.splice(start..end, replacement);
        let serialized_size = bytes.len() as u32;
        set_u32(bytes, 8, serialized_size);
    }

    fn remove_table(bytes: &mut Vec<u8>, id: u16) {
        let (start, end) = table_range(bytes, id);
        bytes.drain(start..end);
        let serialized_size = bytes.len() as u32;
        set_u32(bytes, 8, serialized_size);
    }

    fn set_table_value(bytes: &mut [u8], id: u16, index: usize, value: u32) {
        let (start, _) = table_range(bytes, id);
        let width = read_u16(bytes, start + 2) as usize;
        let offset = start + TABLE_HEADER_SIZE + index * width;
        match width {
            1 => bytes[offset] = value as u8,
            2 => set_u16(bytes, offset, value as u16),
            4 => set_u32(bytes, offset, value),
            _ => unreachable!(),
        }
    }

    fn assert_errno(result: Result<()>, expected: Errno) {
        assert_eq!(result.err().unwrap().error(), expected);
    }

    fn demo_dfa() -> Vec<u8> {
        let blob = include_bytes!(
            "../../../../../test/initramfs/src/regression/security/lsm/apparmor-fixtures/demo.bin"
        );
        let start = blob
            .windows(4)
            .position(|bytes| bytes == MAGIC.to_be_bytes())
            .unwrap();
        let len = read_u32(blob, start + 8) as usize;
        blob[start..start + len].to_vec()
    }

    #[ktest]
    fn recovers_only_the_literal_attachment_language() {
        let bytes = demo_dfa();
        assert_eq!(super::literal_path(&bytes, 13).unwrap(), "/usr/bin/demo");
        assert_eq!(
            super::literal_path(&bytes, 12).unwrap_err().error(),
            Errno::EPROTO
        );

        // An accepting start would also admit the empty string.
        let mut early_accept = bytes.clone();
        set_table_value(&mut early_accept, ACCEPT, 1, 1);
        assert_eq!(
            super::literal_path(&early_accept, 13).unwrap_err().error(),
            Errno::EOPNOTSUPP
        );

        // A nonzero legacy permission is not necessarily MAY_EXEC.
        let mut bad_accept = bytes.clone();
        set_table_value(&mut bad_accept, ACCEPT, 14, 2);
        assert_eq!(
            super::literal_path(&bad_accept, 13).unwrap_err().error(),
            Errno::EOPNOTSUPP
        );

        let mut looped = bytes.clone();
        set_table_value(&mut looped, NEXT, b'/' as usize, 1);
        assert_eq!(
            super::literal_path(&looped, 13).unwrap_err().error(),
            Errno::EOPNOTSUPP
        );

        let mut forked = bytes.clone();
        set_table_value(&mut forked, CHECK, b'!' as usize, 1);
        set_table_value(&mut forked, NEXT, b'!' as usize, 2);
        assert_eq!(
            super::literal_path(&forked, 13).unwrap_err().error(),
            Errno::EOPNOTSUPP
        );

        let mut bad_sink = bytes;
        set_table_value(&mut bad_sink, DEFAULT, 0, 1);
        assert_eq!(
            super::literal_path(&bad_sink, 13).unwrap_err().error(),
            Errno::EPROTO
        );
    }

    #[ktest]
    fn accepts_minimal_dfa_and_nonzero_table_padding() {
        let bytes = minimal_dfa();
        validate(&bytes).unwrap();

        let mut nonzero_padding = bytes;
        let (accept, _) = table_range(&nonzero_padding, ACCEPT);
        nonzero_padding[accept + TABLE_HEADER_SIZE + 2 * 4] = 0xff;
        validate(&nonzero_padding).unwrap();
    }

    #[ktest]
    fn rejects_malformed_table_set_header() {
        let valid = minimal_dfa();
        assert_errno(validate(&valid[..13]), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_u32(&mut bytes, 0, 0);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_u32(&mut bytes, 4, 8);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_u32(&mut bytes, 4, 15);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        let oversized_header = bytes.len() as u32 + 8;
        set_u32(&mut bytes, 4, oversized_header);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_u32(&mut bytes, 8, 0);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_u16(&mut bytes, 12, 1);
        assert_errno(validate(&bytes), Errno::EOPNOTSUPP);

        let mut bytes = valid.clone();
        set_u16(&mut bytes, 12, 2);
        assert_errno(validate(&bytes), Errno::EOPNOTSUPP);

        let mut bytes = valid;
        set_u16(&mut bytes, 12, 4);
        assert_errno(validate(&bytes), Errno::EPROTO);
    }

    #[ktest]
    fn rejects_malformed_table_framing() {
        let valid = minimal_dfa();

        let mut bytes = valid.clone();
        let (accept, _) = table_range(&bytes, ACCEPT);
        set_u16(&mut bytes, accept, 0);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        let (accept, _) = table_range(&bytes, ACCEPT);
        set_u16(&mut bytes, accept + 2, 3);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        let (accept, _) = table_range(&bytes, ACCEPT);
        set_u32(&mut bytes, accept + 4, 1);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        let (accept, _) = table_range(&bytes, ACCEPT);
        set_u32(&mut bytes, accept + 8, 0);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        let (accept2, _) = table_range(&bytes, ACCEPT2);
        set_u16(&mut bytes, accept2, ACCEPT);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        bytes.pop();
        let serialized_size = bytes.len() as u32;
        set_u32(&mut bytes, 8, serialized_size);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid;
        bytes.push(0);
        let serialized_size = bytes.len() as u32;
        set_u32(&mut bytes, 8, serialized_size);
        assert_errno(validate(&bytes), Errno::EPROTO);
    }

    #[ktest]
    fn rejects_missing_or_unsupported_tables_and_shapes() {
        let valid = minimal_dfa();

        let mut bytes = valid.clone();
        remove_table(&mut bytes, ACCEPT2);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        let (accept, _) = table_range(&bytes, ACCEPT);
        set_u16(&mut bytes, accept, META);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        let (accept, _) = table_range(&bytes, ACCEPT);
        set_u16(&mut bytes, accept, EC);
        assert_errno(validate(&bytes), Errno::EOPNOTSUPP);

        let mut bytes = valid.clone();
        replace_table(&mut bytes, DEFAULT, 4, &[0, 0]);
        assert_errno(validate(&bytes), Errno::EOPNOTSUPP);

        let mut bytes = valid.clone();
        replace_table(&mut bytes, ACCEPT, 2, &[0, 0]);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        replace_table(&mut bytes, DEFAULT, 1, &[0, 0]);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        replace_table(&mut bytes, NEXT, 4, &vec![0; 256]);
        assert_errno(validate(&bytes), Errno::EOPNOTSUPP);

        let mut bytes = valid.clone();
        replace_table(&mut bytes, CHECK, 4, &vec![0; 256]);
        assert_errno(validate(&bytes), Errno::EOPNOTSUPP);

        let mut bytes = valid.clone();
        replace_table(&mut bytes, ACCEPT2, 4, &[0]);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid;
        replace_table(&mut bytes, NEXT, 2, &vec![0; 255]);
        assert_errno(validate(&bytes), Errno::EPROTO);
    }

    #[ktest]
    fn rejects_out_of_bounds_or_encoded_states() {
        let valid = minimal_dfa();

        let mut bytes = valid.clone();
        set_table_value(&mut bytes, DEFAULT, 0, 2);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_table_value(&mut bytes, NEXT, 0, 2);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_table_value(&mut bytes, CHECK, 0, 2);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_table_value(&mut bytes, BASE, 0, 1);
        assert_errno(validate(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        set_table_value(&mut bytes, BASE, 0, 0x8000_0000);
        assert_errno(validate(&bytes), Errno::EOPNOTSUPP);

        let mut bytes = valid.clone();
        set_table_value(&mut bytes, BASE, 0, 0x2000_0000);
        assert_errno(validate(&bytes), Errno::EOPNOTSUPP);

        let mut bytes = valid;
        set_table_value(&mut bytes, BASE, 0, 0x4000_0000);
        assert_errno(validate(&bytes), Errno::EPROTO);
    }
}
