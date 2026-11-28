// SPDX-License-Identifier: MPL-2.0

//! AppArmor 5.0.2 policy wire decoding and the kernel's load admission check.
//!
//! The wire decoder checks structural integrity before the admission check
//! decides whether the kernel can use the resulting policy.
//!
//! Field order follows AppArmor 5.0.2's [serializer][parser]. The effective
//! mode, signal range and permission checks follow Linux's [unpacker][linux].
//!
//! [parser]: https://gitlab.com/apparmor/apparmor/-/blob/v5.0.2/parser/parser_interface.c
//! [linux]: https://github.com/torvalds/linux/blob/db2ddb87143519e20a95aa36c60b36107b736a58/security/apparmor/policy_unpack.c

use alloc::{string::String, vec::Vec};
use core::{ops::Range, str};

use aster_core::prelude::*;

use super::{Attachment, Profile, ProfileMode, RuntimePolicy, dfa};

#[cfg(feature = "policy-abi-conformance")]
#[path = "abi/selftest.rs"]
mod selftest;

#[cfg(feature = "policy-abi-conformance")]
pub(crate) fn run_field_tests() {
    selftest::run();
}

const LOAD_ABI_WORD: u32 = 5;
const MAX_MAPPED_SIGNAL: u32 = 35;
const EXEC_TYPE_MASK: u32 = 0x0c00_0000;
const EXEC_TYPE_NAMED: u32 = 0x0800_0000;
const EXEC_INDEX_MASK: u32 = 0x00ff_ffff;

pub(super) fn decode(blob: Vec<u8>) -> Result<Profile> {
    let record = decode_record(&blob)?;
    let name = validate_profile_name(record.name)?;
    let mode = record.mode;
    let runtime = record.runtime_policy(&blob);
    Profile::new(name, mode, blob, runtime)
}

fn decode_record(bytes: &[u8]) -> Result<DecodedProfile<'_>> {
    if bytes.is_empty() {
        return Err(Error::new(Errno::EINVAL));
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.named_u32(b"version")? != LOAD_ABI_WORD {
        return Err(Error::new(Errno::EPROTONOSUPPORT));
    }
    if cursor.peek_name()? == Some(b"namespace") {
        return Err(unsupported_error());
    }
    let record = decode_profile(&mut cursor)?;
    if !cursor.is_eof() {
        return if cursor.peek_name()? == Some(b"version") {
            Err(unsupported_error())
        } else {
            Err(protocol_error())
        };
    }
    Ok(record)
}

#[derive(Debug)]
struct DecodedProfile<'a> {
    name: &'a [u8],
    mode: ProfileMode,
    flags: u32,
    audit: u32,
    path_flags: Option<u32>,
    disconnected: Option<&'a [u8]>,
    disconnected_ipc: Option<&'a [u8]>,
    kill: Option<u32>,
    error: Option<u32>,
    capabilities: DecodedCapabilities,
    attachment: Option<(PolicyDb, usize)>,
    identities: Option<Range<usize>>,
    xattrs: Option<Range<usize>>,
    rlimits: Option<Range<usize>>,
    legacy_network: Option<Range<usize>>,
    policy: Option<PolicyDb>,
    file: Option<PolicyDb>,
}

#[derive(Debug)]
struct DecodedCapabilities {
    allow: u64,
    audit: u64,
    quiet: u64,
    reserved: u64,
    has_upper: bool,
}

#[derive(Debug)]
struct PolicyDb {
    dfa: Range<usize>,
    permissions: Option<Range<usize>>,
    start: Option<u32>,
    file_start: Option<u32>,
    transitions: Option<Range<usize>>,
}

impl PolicyDb {
    fn is_legacy(&self) -> bool {
        self.permissions.is_none()
            && self.start.is_none()
            && self.file_start.is_none()
            && self.transitions.is_none()
    }
}

impl DecodedProfile<'_> {
    fn runtime_policy(self, bytes: &[u8]) -> Option<RuntimePolicy> {
        if !matches!(self.mode, ProfileMode::Enforce | ProfileMode::Complain)
            || self.flags != 0
            || self.audit != 0
            || self.path_flags.is_some()
            || self.disconnected.is_some()
            || self.disconnected_ipc.is_some()
            || self.kill.is_some()
            || self.error.is_some()
            || self.capabilities.allow != 0
            || self.capabilities.audit != 0
            || self.capabilities.quiet != 0
            || self.capabilities.reserved != 0
            || !self.capabilities.has_upper
            || self.identities.is_some()
            || self.xattrs.is_some()
            || self.rlimits.is_some()
            || self.legacy_network.is_some()
            || self.policy.is_some()
            || self.file.is_some()
        {
            return None;
        }
        let attachment = if let Some((db, minimum_len)) = self.attachment {
            if !db.is_legacy()
                || minimum_len > db.dfa.len()
                || dfa::validate_active_attachment(&bytes[db.dfa.clone()]).is_err()
            {
                return None;
            }
            Some(Attachment {
                range: db.dfa,
                minimum_len,
            })
        } else {
            None
        };
        Some(RuntimePolicy { attachment })
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Code {
    U8 = 0,
    U16 = 1,
    U32 = 2,
    U64 = 3,
    Name = 4,
    String = 5,
    Blob = 6,
    Struct = 7,
    StructEnd = 8,
    List = 9,
    ListEnd = 10,
    Array = 11,
    ArrayEnd = 12,
}

impl TryFrom<u8> for Code {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::U8),
            1 => Ok(Self::U16),
            2 => Ok(Self::U32),
            3 => Ok(Self::U64),
            4 => Ok(Self::Name),
            5 => Ok(Self::String),
            6 => Ok(Self::Blob),
            7 => Ok(Self::Struct),
            8 => Ok(Self::StructEnd),
            9 => Ok(Self::List),
            10 => Ok(Self::ListEnd),
            11 => Ok(Self::Array),
            12 => Ok(Self::ArrayEnd),
            _ => Err(protocol_error()),
        }
    }
}

#[derive(Clone, Copy)]
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

enum ArrayLayout {
    Bare,
    Wrapped,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos == self.bytes.len()
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).ok_or_else(protocol_error)?;
        let bytes = self.bytes.get(self.pos..end).ok_or_else(protocol_error)?;
        self.pos = end;
        Ok(bytes)
    }

    fn code(&mut self) -> Result<Code> {
        Code::try_from(self.take(1)?[0])
    }

    fn u16_le(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().map_err(|_| protocol_error())?,
        ))
    }

    fn u32_le(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| protocol_error())?,
        ))
    }

    fn name(&mut self) -> Result<&'a [u8]> {
        let value = self.terminated_chunk(Code::Name)?;
        if value.is_empty() || value.contains(&0) {
            return Err(protocol_error());
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<&'a [u8]> {
        let value = self.string_chunk()?;
        if value.is_empty() || value.contains(&0) {
            return Err(protocol_error());
        }
        Ok(value)
    }

    fn string_chunk(&mut self) -> Result<&'a [u8]> {
        self.terminated_chunk(Code::String)
    }

    fn terminated_chunk(&mut self, expected: Code) -> Result<&'a [u8]> {
        self.expect_code(expected)?;
        let len = usize::from(self.u16_le()?);
        let chunk = self.take(len)?;
        let Some((&0, value)) = chunk.split_last() else {
            return Err(protocol_error());
        };
        Ok(value)
    }

    fn expect_name(&mut self, expected: &[u8]) -> Result<()> {
        if self.name()? == expected {
            Ok(())
        } else {
            Err(unsupported_error())
        }
    }

    fn expect_code(&mut self, expected: Code) -> Result<()> {
        if self.code()? == expected {
            Ok(())
        } else {
            Err(protocol_error())
        }
    }

    fn u32(&mut self) -> Result<u32> {
        self.expect_code(Code::U32)?;
        self.u32_le()
    }

    fn u64(&mut self) -> Result<u64> {
        self.expect_code(Code::U64)?;
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| protocol_error())?,
        ))
    }

    fn named_u32(&mut self, name: &[u8]) -> Result<u32> {
        self.expect_name(name)?;
        self.u32()
    }

    fn named_struct(&mut self, name: &[u8]) -> Result<()> {
        self.expect_name(name)?;
        self.expect_code(Code::Struct)
    }

    fn dfa_blob(&mut self) -> Result<Range<usize>> {
        self.expect_name(b"aadfa")?;
        self.expect_code(Code::Blob)?;
        let len = self.u32_le()? as usize;
        let start = self.pos;
        self.take(len)?;
        // Each serialized DFA ends on the same alignment as its profile.
        let padding = len % 8;
        if padding == len {
            return Err(protocol_error());
        }
        Ok(start + padding..self.pos)
    }

    fn peek_name(&self) -> Result<Option<&'a [u8]>> {
        if self.bytes.get(self.pos) != Some(&(Code::Name as u8)) {
            return Ok(None);
        }
        let mut cursor = *self;
        cursor.name().map(Some)
    }

    fn optional_u32(&mut self, name: &[u8]) -> Result<Option<u32>> {
        if self.peek_name()? == Some(name) {
            Ok(Some(self.named_u32(name)?))
        } else {
            Ok(None)
        }
    }

    fn optional_string(&mut self, name: &[u8]) -> Result<Option<&'a [u8]>> {
        if self.peek_name()? == Some(name) {
            self.expect_name(name)?;
            let value = self.string_chunk()?;
            if value.contains(&0) {
                return Err(protocol_error());
            }
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn array(&mut self) -> Result<usize> {
        self.expect_code(Code::Array)?;
        Ok(usize::from(self.u16_le()?))
    }

    fn optional_array(
        &mut self,
        name: &[u8],
        layout: ArrayLayout,
        parse_fn: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Option<Range<usize>>> {
        if self.peek_name()? != Some(name) {
            return Ok(None);
        }
        let start = self.pos;
        if matches!(layout, ArrayLayout::Wrapped) {
            self.named_struct(name)?;
        } else {
            self.expect_name(name)?;
        }
        parse_fn(self)?;
        self.expect_code(Code::ArrayEnd)?;
        if matches!(layout, ArrayLayout::Wrapped) {
            self.expect_code(Code::StructEnd)?;
        }
        Ok(Some(start..self.pos))
    }
}

fn decode_profile<'a>(cursor: &mut Cursor<'a>) -> Result<DecodedProfile<'a>> {
    cursor.named_struct(b"profile")?;
    let name = cursor.string()?;
    let attachment = if let Some(db) = decode_db(cursor, DbKind::Attachment)? {
        Some((db, cursor.u32()? as usize))
    } else {
        None
    };
    let disconnected = cursor.optional_string(b"disconnected")?;
    let disconnected_ipc = cursor.optional_string(b"disconnected_ipc")?;
    let kill = cursor.optional_u32(b"kill")?;
    // Linux's AppArmor ABI uses its signal mapping, whose last value is 35.
    if kill.is_some_and(|signal| !(1..=MAX_MAPPED_SIGNAL).contains(&signal)) {
        return Err(protocol_error());
    }
    let error = cursor.optional_u32(b"error")?;
    cursor.named_struct(b"flags")?;
    let flags = cursor.u32()?;
    let mode = ProfileMode::from_packed(cursor.u32()?).ok_or_else(protocol_error)?;
    let audit = cursor.u32()?;
    cursor.expect_code(Code::StructEnd)?;
    let path_flags = cursor.optional_u32(b"path_flags")?;
    let low = decode_capability_words(cursor)?;
    let has_upper = cursor.peek_name()? == Some(b"caps64");
    let high = if has_upper {
        cursor.named_struct(b"caps64")?;
        let words = decode_capability_words(cursor)?;
        cursor.expect_code(Code::StructEnd)?;
        words
    } else {
        [0; 4]
    };
    let [allow, audit_caps, quiet, reserved] =
        core::array::from_fn(|i| u64::from(low[i]) | (u64::from(high[i]) << 32));
    let capabilities = DecodedCapabilities {
        allow,
        audit: audit_caps,
        quiet,
        reserved,
        has_upper,
    };
    let identities = cursor.optional_array(b"identity", ArrayLayout::Bare, decode_strings)?;
    let xattrs = cursor.optional_array(b"xattrs", ArrayLayout::Wrapped, decode_strings)?;
    let rlimits = cursor.optional_array(b"rlimits", ArrayLayout::Wrapped, |cursor| {
        let mask = cursor.u32()?;
        let count = cursor.array()?;
        if count > 16 || (mask >> count) != 0 {
            return Err(protocol_error());
        }
        for _ in 0..count {
            cursor.u64()?;
        }
        Ok(())
    })?;
    let legacy_network = cursor.optional_array(b"net_allowed_af", ArrayLayout::Bare, |cursor| {
        let count = cursor.array()?;
        for _ in 0..count {
            for _ in 0..3 {
                cursor.expect_code(Code::U16)?;
                cursor.take(2)?;
            }
        }
        Ok(())
    })?;
    let policy = if cursor.peek_name()? == Some(b"policydb") {
        cursor.named_struct(b"policydb")?;
        let db = decode_db(cursor, DbKind::Policy)?.ok_or_else(protocol_error)?;
        cursor.expect_code(Code::StructEnd)?;
        Some(db)
    } else {
        None
    };
    let file = decode_db(cursor, DbKind::File)?;
    if cursor.peek_name()?.is_some() {
        return Err(unsupported_error());
    }
    cursor.expect_code(Code::StructEnd)?;
    Ok(DecodedProfile {
        name,
        mode,
        flags,
        audit,
        path_flags,
        disconnected,
        disconnected_ipc,
        kill,
        error,
        capabilities,
        attachment,
        identities,
        xattrs,
        rlimits,
        legacy_network,
        policy,
        file,
    })
}

fn decode_strings(cursor: &mut Cursor<'_>) -> Result<()> {
    for _ in 0..cursor.array()? {
        cursor.string()?;
    }
    Ok(())
}

fn decode_capability_words(cursor: &mut Cursor<'_>) -> Result<[u32; 4]> {
    Ok([cursor.u32()?, cursor.u32()?, cursor.u32()?, cursor.u32()?])
}

#[derive(Clone, Copy)]
enum DbKind {
    Attachment,
    Policy,
    File,
}

fn decode_db(cursor: &mut Cursor<'_>, kind: DbKind) -> Result<Option<PolicyDb>> {
    let mut table = None;
    let mut count = 0;
    let mut max_target = None;
    if cursor.peek_name()? == Some(b"perms") {
        cursor.named_struct(b"perms")?;
        if cursor.named_u32(b"version")? != 1 {
            return Err(unsupported_error());
        }
        count = cursor.array()?;
        if count == 0 {
            return Err(protocol_error());
        }
        let start = cursor.pos;
        for _ in 0..count {
            let mut words = [0; 14];
            for word in &mut words {
                *word = cursor.u32()?;
            }
            let [
                _,
                allow,
                deny,
                subtree,
                cond,
                kill,
                complain,
                prompt,
                _,
                _,
                hide,
                xindex,
                tag,
                label,
            ] = words;
            if (allow & deny)
                | (subtree & !allow)
                | (cond & (allow | deny))
                | (kill & allow)
                | ((complain | prompt) & (allow | deny))
                | (complain & prompt)
                | (hide & allow)
                != 0
            {
                return Err(protocol_error());
            }
            // Policy tags are not emitted by the pinned parser.
            if tag != 0 {
                return Err(unsupported_error());
            }
            if xindex & EXEC_TYPE_MASK == EXEC_TYPE_NAMED {
                let index = (xindex & EXEC_INDEX_MASK) as usize;
                max_target = Some(max_target.map_or(index, |old: usize| old.max(index)));
            }
            if label != 0 {
                let index = label as usize;
                max_target = Some(max_target.map_or(index, |old| old.max(index)));
            }
        }
        table = Some(start..cursor.pos);
        cursor.expect_code(Code::ArrayEnd)?;
        cursor.expect_code(Code::StructEnd)?;
    }
    let permission_format = match cursor.optional_u32(b"permsv")? {
        None => dfa::PermissionFormat::Indexed,
        Some(3) if table.is_some() => dfa::PermissionFormat::OwnerPairs,
        Some(_) => return Err(unsupported_error()),
    };
    if cursor.peek_name()? != Some(b"aadfa") {
        if table.is_some() {
            return Err(protocol_error());
        }
        return Ok(None);
    }
    let dfa = cursor.dfa_blob()?;
    let start = cursor.optional_u32(b"start")?;
    let file_start = cursor.optional_u32(b"dfa_start")?;
    let start_pos = cursor.pos;
    let mut transition_count = 0;
    let transitions = if cursor.peek_name()? == Some(b"xtable") {
        cursor.named_struct(b"xtable")?;
        transition_count = cursor.array()?;
        for _ in 0..transition_count {
            let target = cursor.string_chunk()?;
            if target.is_empty() || matches!(target[0], b' ' | b'\t'..=b'\r') {
                return Err(protocol_error());
            }
            if target.starts_with(b":") {
                let sep = target
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(protocol_error)?;
                if sep < 2
                    || sep + 1 == target.len()
                    || matches!(target[sep + 1], b' ' | b'\t'..=b'\r')
                    || target[sep + 1..].contains(&0)
                {
                    return Err(protocol_error());
                }
            } else if target.contains(&0) {
                return Err(protocol_error());
            }
        }
        cursor.expect_code(Code::ArrayEnd)?;
        cursor.expect_code(Code::StructEnd)?;
        Some(start_pos..cursor.pos)
    } else {
        None
    };
    if max_target.is_some_and(|index| index >= transition_count) {
        return Err(protocol_error());
    }
    dfa::validate_policy(
        &cursor.bytes[dfa.clone()],
        table.as_ref().map(|_| (count, permission_format)),
        start.unwrap_or(1),
        file_start.unwrap_or(1),
        matches!(kind, DbKind::File).then_some(transition_count),
    )?;
    Ok(Some(PolicyDb {
        dfa,
        permissions: table,
        start,
        file_start,
        transitions,
    }))
}

pub(super) fn validate_profile_name(raw: &[u8]) -> Result<String> {
    let name = str::from_utf8(raw).map_err(|_| Error::new(Errno::EILSEQ))?;
    if name.starts_with(':') || name.contains("//") {
        return Err(unsupported_error());
    }
    if raw.is_empty()
        || raw.len() > super::MAX_PROFILE_NAME_LEN
        || name.chars().any(char::is_control)
    {
        return Err(Error::new(Errno::EINVAL));
    }

    let mut owned = String::new();
    owned
        .try_reserve_exact(name.len())
        .map_err(|_| Error::new(Errno::ENOMEM))?;
    owned.push_str(name);
    Ok(owned)
}

fn protocol_error() -> Error {
    Error::new(Errno::EPROTO)
}

fn unsupported_error() -> Error {
    Error::new(Errno::EOPNOTSUPP)
}
