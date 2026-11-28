// SPDX-License-Identifier: MPL-2.0

//! Validation of AppArmor policy DFA encodings and matching of active attachments.
//!
//! Table layout follows the official parser's [`flex-tables.h`][format]. State
//! bounds follow Linux's [`verify_dfa`][verify]. Validation borrows the serialized
//! tables; the enclosing profile retains the original bytes without copying them.
//!
//! [format]: https://gitlab.com/apparmor/apparmor/-/blob/v5.0.2/parser/libapparmor_re/flex-tables.h
//! [verify]: https://github.com/torvalds/linux/blob/db2ddb87143519e20a95aa36c60b36107b736a58/security/apparmor/match.c

use alloc::vec::Vec;

use aster_core::prelude::*;

const MAGIC: u32 = 0x1b5e_783d;
const HEADER_SIZE: usize = 14;
const TABLE_HEADER_SIZE: usize = 12;
const ACCEPT: usize = 0;
const BASE: usize = 1;
const CHECK: usize = 2;
const DEFAULT: usize = 3;
const EQUIVALENCE: usize = 4;
const ACCEPT2: usize = 6;
const NEXT: usize = 7;
const TABLE_ALIGNMENT: usize = 8;
const CHARACTER_COUNT: usize = 256;
const ENCODING_FLAGS_MASK: u16 = 3;
const STATE_FLAGS_MASK: u32 = 0xff00_0000;
const DIFF_STATE: u32 = 0x8000_0000;
const OOB_STATE: u32 = 0x2000_0000;
const DIFF_ENCODING: u16 = 1;
const OOB_ENCODING: u16 = 2;
const ATTACHMENT_EXEC_PERMISSION: u32 = 1;
const EXEC_TYPE_SHIFTS: [u32; 2] = [10, 24];
const EXEC_TYPE_MASK: u32 = 0xf;
const NAMED_EXEC_TRANSITION_MIN: u32 = 4;
const START_STATE: u32 = 1;

#[derive(Clone, Copy)]
enum Encoding {
    Ordinary,
    Extended,
}

enum SecondAcceptTable {
    Required,
    Optional,
}

#[derive(Clone, Copy)]
pub(super) enum PermissionFormat {
    Indexed,
    OwnerPairs,
}

pub(super) fn match_attachment(bytes: &[u8], path: &[u8]) -> Result<Option<usize>> {
    // Load-time validation established the state bounds; decode just the table views.
    let tables = decode_tables(bytes, Encoding::Ordinary)?;
    let base = required_table(&tables, BASE)?;
    let check = required_table(&tables, CHECK)?;
    let next = required_table(&tables, NEXT)?;
    let default = required_table(&tables, DEFAULT)?;
    let accept = required_table(&tables, ACCEPT)?;
    let mut state = START_STATE;
    // Linux's left-match priority counts the prefix before the first DFA loop.
    let mut history = [0; 32];
    let mut history_pos = 0;
    let mut history_len = 0;
    let mut prefix_len = 0;
    let mut loop_found = false;
    for byte in path {
        if !loop_found {
            history[history_pos] = state;
        }
        let character = if let Some(equivalence) = tables[EQUIVALENCE] {
            equivalence.get(usize::from(*byte))?
        } else {
            u32::from(*byte)
        };
        let position = base
            .get(state as usize)?
            .checked_add(character)
            .ok_or_else(protocol_error)? as usize;
        state = if check.get(position)? == state {
            next.get(position)?
        } else {
            default.get(state as usize)?
        };
        if !loop_found {
            if history[history_pos] >= state {
                for adjustment in 0..history_len {
                    let position = history_pos.wrapping_sub(adjustment) & 31;
                    if history[position] == state {
                        prefix_len -= adjustment;
                        loop_found = true;
                        break;
                    }
                }
            }
            if !loop_found {
                history_pos = (history_pos + 1) & 31;
                history_len = (history_len + 1).min(history.len());
                prefix_len += 1;
            }
        }
    }
    Ok((accept.get(state as usize)? & ATTACHMENT_EXEC_PERMISSION != 0).then_some(prefix_len))
}

/// Checks the restrictions imposed on an already validated attachment DFA.
pub(super) fn validate_active_attachment(bytes: &[u8]) -> Result<()> {
    let tables = decode_tables(bytes, Encoding::Ordinary)?;
    let accept = required_table(&tables, ACCEPT)?;
    let accept2 = required_table(&tables, ACCEPT2)?;
    for state in 0..accept.len() {
        // Attachment matching consumes only the legacy AA_MAY_EXEC permission.
        if accept.get(state)? & !ATTACHMENT_EXEC_PERMISSION != 0 || accept2.get(state)? != 0 {
            return Err(unsupported_error());
        }
    }
    Ok(())
}

/// Checks wire indexes without enabling evaluation of the policy.
pub(super) fn validate_policy(
    bytes: &[u8],
    permissions: Option<(usize, PermissionFormat)>,
    start: u32,
    file_start: u32,
    file_transitions: Option<usize>,
) -> Result<()> {
    let second_accept = match permissions {
        Some((_, PermissionFormat::Indexed)) => SecondAcceptTable::Optional,
        _ => SecondAcceptTable::Required,
    };
    let tables = validate_tables(bytes, second_accept)?;
    let accept = required_table(&tables, ACCEPT)?;
    if start as usize >= accept.len() || file_start as usize >= accept.len() {
        return Err(protocol_error());
    }
    for state in 0..accept.len() {
        let value = accept.get(state)?;
        if let Some((count, format)) = permissions {
            let owner = if matches!(format, PermissionFormat::OwnerPairs) {
                required_table(&tables, ACCEPT2)?.get(state)?
            } else {
                0
            };
            if owner > 1 || value as usize >= count.saturating_sub(owner as usize) {
                return Err(protocol_error());
            }
        } else if let Some(count) = file_transitions {
            for shift in EXEC_TYPE_SHIFTS {
                let index = (value >> shift) & EXEC_TYPE_MASK;
                if index >= NAMED_EXEC_TRANSITION_MIN
                    && (index - NAMED_EXEC_TRANSITION_MIN) as usize >= count
                {
                    return Err(protocol_error());
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Table<'a> {
    bytes: &'a [u8],
    width: usize,
}

impl Table<'_> {
    fn len(&self) -> usize {
        self.bytes.len() / self.width
    }

    fn get(&self, index: usize) -> Result<u32> {
        let start = index.checked_mul(self.width).ok_or_else(protocol_error)?;
        let end = start.checked_add(self.width).ok_or_else(protocol_error)?;
        let bytes = self.bytes.get(start..end).ok_or_else(protocol_error)?;
        Ok(bytes
            .iter()
            .fold(0, |value, byte| (value << 8) | u32::from(*byte)))
    }
}

fn validate_tables(
    bytes: &[u8],
    second_accept: SecondAcceptTable,
) -> Result<[Option<Table<'_>>; 8]> {
    let tables = decode_tables(bytes, Encoding::Extended)?;
    let encoding_flags = read_u16(bytes, 12)?;
    let accept = required_table(&tables, ACCEPT)?;
    let accept2 = tables[ACCEPT2];
    let base = required_table(&tables, BASE)?;
    let default = required_table(&tables, DEFAULT)?;
    let next = required_table(&tables, NEXT)?;
    let check = required_table(&tables, CHECK)?;
    let states = base.len();
    let transitions = next.len();
    if states < 2
        || accept.len() != states
        || (matches!(second_accept, SecondAcceptTable::Required) && accept2.is_none())
        || accept2.is_some_and(|table| table.len() != states)
        || default.len() != states
        || check.len() != transitions
        || tables[EQUIVALENCE].is_some_and(|table| table.len() != CHARACTER_COUNT)
    {
        return Err(protocol_error());
    }

    for state in 0..states {
        if default.get(state)? as usize >= states {
            return Err(protocol_error());
        }
        let packed = base.get(state)?;
        let state_flags = packed & STATE_FLAGS_MASK;
        let index = packed & !STATE_FLAGS_MASK;
        if state_flags & !(DIFF_STATE | OOB_STATE) != 0
            || (state_flags & DIFF_STATE != 0 && encoding_flags & DIFF_ENCODING == 0)
            || (state_flags & OOB_STATE != 0 && (encoding_flags & OOB_ENCODING == 0 || index == 0))
            || index as usize + CHARACTER_COUNT > transitions
        {
            return Err(protocol_error());
        }
    }
    for slot in 0..transitions {
        if next.get(slot)? as usize >= states || check.get(slot)? as usize >= states {
            return Err(protocol_error());
        }
    }
    if encoding_flags & DIFF_ENCODING != 0 {
        validate_diff_chains(base, default)?;
    }
    Ok(tables)
}

fn validate_diff_chains(base: Table<'_>, default: Table<'_>) -> Result<()> {
    let mut marks = Vec::new();
    marks
        .try_reserve_exact(base.len())
        .map_err(|_| Error::new(Errno::ENOMEM))?;
    marks.resize(base.len(), 0_u8);
    // A diff state falls back to its default; every such chain must terminate.
    for origin in 0..base.len() {
        let mut state = origin;
        while base.get(state)? & DIFF_STATE != 0 {
            match marks[state] {
                1 => return Err(protocol_error()),
                2 => break,
                _ => marks[state] = 1,
            }
            state = default.get(state)? as usize;
        }
        state = origin;
        while marks[state] == 1 {
            marks[state] = 2;
            state = default.get(state)? as usize;
        }
    }
    Ok(())
}

fn decode_tables(bytes: &[u8], encoding: Encoding) -> Result<[Option<Table<'_>>; 8]> {
    if bytes.len() < HEADER_SIZE || read_u32(bytes, 0)? != MAGIC {
        return Err(protocol_error());
    }
    let header_size = read_u32(bytes, 4)? as usize;
    let size = read_u32(bytes, 8)? as usize;
    if header_size < HEADER_SIZE
        || !header_size.is_multiple_of(TABLE_ALIGNMENT)
        || header_size > size
        || size != bytes.len()
    {
        return Err(protocol_error());
    }
    let flags = read_u16(bytes, 12)?;
    if flags & !ENCODING_FLAGS_MASK != 0 {
        return Err(protocol_error());
    }
    if flags != 0 && matches!(encoding, Encoding::Ordinary) {
        return Err(unsupported_error());
    }

    let mut tables = [None; 8];
    let mut offset = header_size;
    while offset < bytes.len() {
        let header_end = offset
            .checked_add(TABLE_HEADER_SIZE)
            .ok_or_else(protocol_error)?;
        let header = bytes.get(offset..header_end).ok_or_else(protocol_error)?;
        let index = usize::from(
            read_u16(header, 0)?
                .checked_sub(1)
                .ok_or_else(protocol_error)?,
        );
        let width = usize::from(read_u16(header, 2)?);
        let valid_width = match index {
            ACCEPT | BASE | ACCEPT2 => width == size_of::<u32>(),
            CHECK | DEFAULT | NEXT => width == size_of::<u16>() || width == size_of::<u32>(),
            EQUIVALENCE => width == size_of::<u8>(),
            _ => false,
        };
        if !valid_width {
            return Err(protocol_error());
        }
        if tables[index].is_some() || read_u32(header, 4)? != 0 {
            return Err(protocol_error());
        }
        let len = read_u32(header, 8)? as usize;
        let data_size = len.checked_mul(width).ok_or_else(protocol_error)?;
        let data_end = header_end
            .checked_add(data_size)
            .ok_or_else(protocol_error)?;
        let table_end = data_end
            .checked_add(TABLE_ALIGNMENT - 1)
            .ok_or_else(protocol_error)?
            & !(TABLE_ALIGNMENT - 1);
        if len == 0 || table_end > bytes.len() {
            return Err(protocol_error());
        }
        tables[index] = Some(Table {
            bytes: bytes.get(header_end..data_end).ok_or_else(protocol_error)?,
            width,
        });
        offset = table_end;
    }
    Ok(tables)
}

fn required_table<'a>(tables: &[Option<Table<'a>>; 8], index: usize) -> Result<Table<'a>> {
    tables[index].ok_or_else(protocol_error)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let end = offset.checked_add(2).ok_or_else(protocol_error)?;
    Ok(u16::from_be_bytes(
        bytes
            .get(offset..end)
            .ok_or_else(protocol_error)?
            .try_into()
            .map_err(|_| protocol_error())?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset.checked_add(4).ok_or_else(protocol_error)?;
    Ok(u32::from_be_bytes(
        bytes
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
