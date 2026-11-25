// SPDX-License-Identifier: MPL-2.0

//! Decoding of AppArmor ABI v5 policy data.
//!
//! Only a single profile with a name, supported mode, and zero capability words
//! is accepted. Other policy fields are rejected, even if valid in the full ABI.
//! The format follows the [AppArmor 5.0.2 serializer][parser] and
//! the [Linux policy decoder][linux].
//!
//! [parser]: https://gitlab.com/apparmor/apparmor/-/blob/v5.0.2/parser/parser_interface.c
//! [linux]: https://github.com/torvalds/linux/blob/db2ddb87143519e20a95aa36c60b36107b736a58/security/apparmor/policy_unpack.c

use alloc::{string::String, vec::Vec};
use core::str;

use aster_core::prelude::*;

use super::{Profile, ProfileMode};

const SUPPORTED_ABI_VERSION: u32 = 5;

pub(super) fn decode(blob: Vec<u8>) -> Result<Profile> {
    if blob.is_empty() {
        return Err(Error::new(Errno::EINVAL));
    }

    let mut cursor = Cursor::new(&blob);
    decode_header(&mut cursor)?;
    let (name, mode) = decode_profile(&mut cursor)?;
    if !cursor.is_eof() {
        return if cursor.peek_name()? == Some(b"version") {
            Err(unsupported_error())
        } else {
            Err(protocol_error())
        };
    }
    Profile::new(name, mode, blob)
}

fn decode_header(cursor: &mut Cursor<'_>) -> Result<()> {
    if cursor.named_u32(b"version")? != SUPPORTED_ABI_VERSION {
        return Err(Error::new(Errno::EPROTONOSUPPORT));
    }
    if cursor.peek_name()? == Some(b"namespace") {
        return Err(unsupported_error());
    }
    Ok(())
}

fn decode_profile(cursor: &mut Cursor<'_>) -> Result<(String, ProfileMode)> {
    cursor.named_struct(b"profile")?;
    let name = validate_profile_name(cursor.string()?)?;
    let mode = decode_flags(cursor)?;
    if cursor.peek_name()? == Some(b"path_flags") {
        return Err(unsupported_error());
    }
    decode_zero_capabilities(cursor)?;
    decode_zero_caps64(cursor)?;
    if cursor.peek_name()?.is_some() {
        return Err(unsupported_error());
    }
    cursor.expect_code(Code::StructEnd)?;

    Ok((name, mode))
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

fn decode_flags(cursor: &mut Cursor<'_>) -> Result<ProfileMode> {
    cursor.expect_no_named_field_before(b"flags")?;
    cursor.named_struct(b"flags")?;
    let flags = cursor.u32()?;
    let mode = cursor.u32()?;
    let audit = cursor.u32()?;
    cursor.expect_code(Code::StructEnd)?;
    if flags != 0 || audit != 0 {
        return Err(unsupported_error());
    }
    ProfileMode::decode(mode)
}

fn decode_zero_capabilities(cursor: &mut Cursor<'_>) -> Result<()> {
    let mut any = 0;
    for _ in 0..4 {
        any |= cursor.u32()?;
    }
    if any == 0 {
        Ok(())
    } else {
        Err(unsupported_error())
    }
}

fn decode_zero_caps64(cursor: &mut Cursor<'_>) -> Result<()> {
    cursor.expect_no_named_field_before(b"caps64")?;
    cursor.named_struct(b"caps64")?;
    decode_zero_capabilities(cursor)?;
    cursor.expect_code(Code::StructEnd)
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
        self.nul_terminated_chunk(Code::Name)
    }

    fn string(&mut self) -> Result<&'a [u8]> {
        self.nul_terminated_chunk(Code::String)
    }

    fn nul_terminated_chunk(&mut self, expected: Code) -> Result<&'a [u8]> {
        self.expect_code(expected)?;
        let len = usize::from(self.u16_le()?);
        let chunk = self.take(len)?;
        let Some((&0, value)) = chunk.split_last() else {
            return Err(protocol_error());
        };
        if value.is_empty() || value.contains(&0) {
            return Err(protocol_error());
        }
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

    fn named_u32(&mut self, name: &[u8]) -> Result<u32> {
        self.expect_name(name)?;
        self.u32()
    }

    fn named_struct(&mut self, name: &[u8]) -> Result<()> {
        self.expect_name(name)?;
        self.expect_code(Code::Struct)
    }

    fn peek_name(&self) -> Result<Option<&'a [u8]>> {
        if self.bytes.get(self.pos) != Some(&(Code::Name as u8)) {
            return Ok(None);
        }
        let mut cursor = *self;
        cursor.name().map(Some)
    }

    fn expect_no_named_field_before(&self, expected: &[u8]) -> Result<()> {
        if self.peek_name()?.is_some_and(|actual| actual != expected) {
            Err(unsupported_error())
        } else {
            Ok(())
        }
    }
}

fn protocol_error() -> Error {
    Error::new(Errno::EPROTO)
}

fn unsupported_error() -> Error {
    Error::new(Errno::EOPNOTSUPP)
}
