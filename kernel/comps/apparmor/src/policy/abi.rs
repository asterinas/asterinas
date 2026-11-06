// SPDX-License-Identifier: MPL-2.0

//! Decoder for the minimal AppArmor policy ABI v5 subset.
//!
//! Type tags and profile field order follow AppArmor 5.0.2's [`sd_code` and
//! `sd_serialize_profile`][parser] and Linux's [`unpack_profile`][linux].
//! This subset accepts a profile name, flags, and zero capability words, including
//! `caps64`. Rejection of other valid ABI fields is a subset limitation.
//!
//! [parser]: https://gitlab.com/apparmor/apparmor/-/blob/v5.0.2/parser/parser_interface.c
//! [linux]: https://github.com/torvalds/linux/blob/db2ddb87143519e20a95aa36c60b36107b736a58/security/apparmor/policy_unpack.c

use alloc::string::String;
use core::str;

use aster_core::prelude::*;

use super::{MAX_PROFILE_NAME_LEN, Profile, ProfileMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
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

pub(super) fn decode(blob: &[u8]) -> Result<Profile> {
    if blob.is_empty() {
        return Err(Error::new(Errno::EINVAL));
    }

    let mut cursor = Cursor::new(blob);
    decode_header(&mut cursor)?;
    let profile = decode_profile(&mut cursor)?;
    if !cursor.is_eof() {
        return if cursor.peek_name()? == Some(b"version") {
            Err(unsupported_error())
        } else {
            Err(protocol_error())
        };
    }
    Ok(profile)
}

fn decode_header(cursor: &mut Cursor<'_>) -> Result<()> {
    if cursor.named_u32(b"version")? != 5 {
        return Err(Error::new(Errno::EPROTONOSUPPORT));
    }
    if cursor.peek_name()? == Some(b"namespace") {
        return Err(unsupported_error());
    }
    Ok(())
}

fn decode_profile(cursor: &mut Cursor<'_>) -> Result<Profile> {
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

    Ok(Profile { name, mode })
}

fn validate_profile_name(raw: &[u8]) -> Result<String> {
    let name = str::from_utf8(raw).map_err(|_| Error::new(Errno::EILSEQ))?;
    if name.starts_with(':') || name.contains("//") {
        return Err(unsupported_error());
    }
    if raw.is_empty() || raw.len() > MAX_PROFILE_NAME_LEN || name.chars().any(char::is_control) {
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

    use super::decode;

    #[derive(Clone)]
    struct UnitOptions<'a> {
        version: u32,
        mode: u32,
        flags: u32,
        audit: u32,
        low_caps: [u32; 4],
        high_caps: [u32; 4],
        namespace: bool,
        before_flags: Option<&'a str>,
        path_flags: Option<u32>,
        before_end: Option<&'a str>,
    }

    impl Default for UnitOptions<'_> {
        fn default() -> Self {
            Self {
                version: 5,
                mode: 0,
                flags: 0,
                audit: 0,
                low_caps: [0; 4],
                high_caps: [0; 4],
                namespace: false,
                before_flags: None,
                path_flags: None,
                before_end: None,
            }
        }
    }

    fn push_u32(output: &mut Vec<u8>, value: u32) {
        output.push(2);
        output.extend_from_slice(&value.to_le_bytes());
    }

    fn push_chunk(output: &mut Vec<u8>, code: u8, value: &[u8]) {
        output.push(code);
        output.extend_from_slice(&((value.len() + 1) as u16).to_le_bytes());
        output.extend_from_slice(value);
        output.push(0);
    }

    fn push_name(output: &mut Vec<u8>, name: &str) {
        push_chunk(output, 4, name.as_bytes());
    }

    fn push_named_u32(output: &mut Vec<u8>, name: &str, value: u32) {
        push_name(output, name);
        push_u32(output, value);
    }

    fn push_named_struct(output: &mut Vec<u8>, name: &str) {
        push_name(output, name);
        output.push(7);
    }

    fn policy_unit(name: &[u8], options: &UnitOptions<'_>) -> Vec<u8> {
        let mut output = Vec::new();
        push_named_u32(&mut output, "version", options.version);
        if options.namespace {
            push_name(&mut output, "namespace");
            push_chunk(&mut output, 5, b"m1-test");
        }
        push_named_struct(&mut output, "profile");
        push_chunk(&mut output, 5, name);
        if let Some(name) = options.before_flags {
            push_named_u32(&mut output, name, 0);
        }
        push_named_struct(&mut output, "flags");
        push_u32(&mut output, options.flags);
        push_u32(&mut output, options.mode);
        push_u32(&mut output, options.audit);
        output.push(8);
        if let Some(path_flags) = options.path_flags {
            push_named_u32(&mut output, "path_flags", path_flags);
        }
        for value in options.low_caps {
            push_u32(&mut output, value);
        }
        push_named_struct(&mut output, "caps64");
        for value in options.high_caps {
            push_u32(&mut output, value);
        }
        output.push(8);
        if let Some(name) = options.before_end {
            push_named_u32(&mut output, name, 0);
        }
        output.push(8);
        output
    }

    fn assert_errno<T>(result: Result<T>, expected: Errno) {
        assert_eq!(result.err().unwrap().error(), expected);
    }

    #[ktest]
    fn decodes_minimal_enforce_and_complain_profiles() {
        let enforce = decode(&policy_unit(b"demo", &UnitOptions::default())).unwrap();
        assert_eq!(enforce.name(), "demo");
        assert_eq!(enforce.mode().as_str(), "enforce");

        let complain = UnitOptions {
            mode: 1,
            ..Default::default()
        };
        let profile = decode(&policy_unit(b"observe", &complain)).unwrap();
        assert_eq!(profile.name(), "observe");
        assert_eq!(profile.mode().as_str(), "complain");
    }

    #[ktest]
    fn rejects_truncation_trailing_bytes_and_multiple_profiles() {
        assert_errno(decode(&[]), Errno::EINVAL);

        let valid = policy_unit(b"first", &UnitOptions::default());
        for end in 1..valid.len() {
            assert_errno(decode(&valid[..end]), Errno::EPROTO);
        }

        let mut trailing = valid.clone();
        trailing.push(0);
        assert_errno(decode(&trailing), Errno::EPROTO);

        let mut multiple = valid;
        multiple.extend_from_slice(&policy_unit(b"second", &UnitOptions::default()));
        assert_errno(decode(&multiple), Errno::EOPNOTSUPP);
    }

    #[ktest]
    fn rejects_bad_codes_lengths_and_nuls() {
        let valid = policy_unit(b"demo", &UnitOptions::default());

        let mut bytes = valid.clone();
        bytes[0] = 0xff;
        assert_errno(decode(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        bytes[1..3].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_errno(decode(&bytes), Errno::EPROTO);

        let mut bytes = valid.clone();
        bytes[10] = b'x';
        assert_errno(decode(&bytes), Errno::EPROTO);

        let mut bytes = valid;
        bytes[3] = 0;
        assert_errno(decode(&bytes), Errno::EPROTO);
    }

    #[ktest]
    fn distinguishes_unsupported_and_invalid_policy_values() {
        let v7 = UnitOptions {
            version: 7,
            ..Default::default()
        };
        assert_errno(decode(&policy_unit(b"demo", &v7)), Errno::EPROTONOSUPPORT);

        let namespace = UnitOptions {
            namespace: true,
            ..Default::default()
        };
        assert_errno(decode(&policy_unit(b"demo", &namespace)), Errno::EOPNOTSUPP);

        for mode in 2..=4 {
            let options = UnitOptions {
                mode,
                ..Default::default()
            };
            assert_errno(decode(&policy_unit(b"demo", &options)), Errno::EOPNOTSUPP);
        }
        let invalid_mode = UnitOptions {
            mode: 5,
            ..Default::default()
        };
        assert_errno(decode(&policy_unit(b"demo", &invalid_mode)), Errno::EPROTO);

        for options in [
            UnitOptions {
                flags: 1,
                ..Default::default()
            },
            UnitOptions {
                audit: 1,
                ..Default::default()
            },
            UnitOptions {
                low_caps: [1, 0, 0, 0],
                ..Default::default()
            },
            UnitOptions {
                high_caps: [0, 0, 0, 1],
                ..Default::default()
            },
            UnitOptions {
                before_flags: Some("aadfa"),
                ..Default::default()
            },
            UnitOptions {
                path_flags: Some(1),
                ..Default::default()
            },
            UnitOptions {
                before_end: Some("policydb"),
                ..Default::default()
            },
        ] {
            assert_errno(decode(&policy_unit(b"demo", &options)), Errno::EOPNOTSUPP);
        }
    }

    #[ktest]
    fn validates_profile_names() {
        let max_name = vec![b'a'; 255];
        assert_eq!(
            decode(&policy_unit(&max_name, &UnitOptions::default()))
                .unwrap()
                .name()
                .as_bytes(),
            max_name.as_slice()
        );

        assert_errno(
            decode(&policy_unit(&[0xff], &UnitOptions::default())),
            Errno::EILSEQ,
        );
        for name in [b":namespace//demo".as_slice(), b"parent//child"] {
            assert_errno(
                decode(&policy_unit(name, &UnitOptions::default())),
                Errno::EOPNOTSUPP,
            );
        }
        assert_errno(
            decode(&policy_unit(b"line\nbreak", &UnitOptions::default())),
            Errno::EINVAL,
        );
        assert_errno(
            decode(&policy_unit(&vec![b'a'; 256], &UnitOptions::default())),
            Errno::EINVAL,
        );
    }
}
