// SPDX-License-Identifier: MPL-2.0

//! AppArmor field reads exercised by the initramfs conformance kernel.
//!
//! The cases follow the [Linux v7.2-rc7 unpacker tests][kunit]. Fixtures use
//! the Rust cursor's contract: a failed field aborts decoding, while an
//! absent optional field leaves the cursor unchanged.
//!
//! [kunit]: https://github.com/torvalds/linux/blob/db2ddb87143519e20a95aa36c60b36107b736a58/security/apparmor/policy_unpack_test.c

use aster_core::prelude::*;

use super::{ArrayLayout, Code, Cursor, ProfileMode, decode_record};

pub(super) fn run() {
    field_bounds();
    field_names_and_tags();
    string_chunks();
    integers();
    arrays();
    blob();
    profile();
    profile_rlimits();
}

fn expect_error<T: core::fmt::Debug>(result: Result<T>, errno: Errno) {
    assert_eq!(result.unwrap_err().error(), errno);
}

fn field_bounds() {
    let data = b"abc";
    let mut cursor = Cursor::new(data);
    assert_eq!(cursor.take(0).unwrap(), b"");
    assert_eq!(cursor.take(1).unwrap(), b"a");
    expect_error(cursor.take(data.len()), Errno::EPROTO);
    assert_eq!(cursor.take(2).unwrap(), b"bc");
    assert!(cursor.is_eof());
    expect_error(cursor.take(1), Errno::EPROTO);
}

fn field_names_and_tags() {
    let field = b"\x04\x06\0value\0\x02\x04\x03\x02\x01";
    let mut cursor = Cursor::new(field);
    assert_eq!(cursor.peek_name().unwrap(), Some(b"value".as_slice()));
    assert_eq!(cursor.optional_u32(b"other").unwrap(), None);
    assert_eq!(cursor.named_u32(b"value").unwrap(), 0x0102_0304);
    assert!(cursor.is_eof());

    expect_error(Cursor::new(field).named_u32(b"other"), Errno::EOPNOTSUPP);
    expect_error(Cursor::new(b"\x05").expect_code(Code::U32), Errno::EPROTO);
    expect_error(Cursor::new(b"").expect_code(Code::U32), Errno::EPROTO);
    expect_error(Cursor::new(b"\x04\x06\0val").peek_name(), Errno::EPROTO);
}

fn string_chunks() {
    let text = b"\x05\x08\0testing\0";
    let mut cursor = Cursor::new(text);
    assert_eq!(cursor.string().unwrap(), b"testing");
    assert!(cursor.is_eof());

    let mut cursor = Cursor::new(b"\x04\x05\0text\0\x05\x08\0testing\0");
    assert_eq!(cursor.optional_string(b"other").unwrap(), None);
    assert_eq!(
        cursor.optional_string(b"text").unwrap(),
        Some(b"testing".as_slice())
    );
    assert!(cursor.is_eof());

    expect_error(Cursor::new(b"\x05\x08").string(), Errno::EPROTO);
    expect_error(Cursor::new(&text[..text.len() - 1]).string(), Errno::EPROTO);
    expect_error(Cursor::new(b"\x05\x04\0text").string(), Errno::EPROTO);
}

fn integers() {
    let u32_bytes = b"\x02\x04\x03\x02\x01";
    assert_eq!(Cursor::new(u32_bytes).u32().unwrap(), 0x0102_0304);
    expect_error(Cursor::new(&u32_bytes[..4]).u32(), Errno::EPROTO);

    let u64_bytes = b"\x03\x08\x07\x06\x05\x04\x03\x02\x01";
    assert_eq!(Cursor::new(u64_bytes).u64().unwrap(), 0x0102_0304_0506_0708);
    expect_error(Cursor::new(&u64_bytes[..8]).u64(), Errno::EPROTO);
    expect_error(Cursor::new(u32_bytes).u64(), Errno::EPROTO);
}

fn arrays() {
    let header = b"\x0b\x02\x01";
    assert_eq!(Cursor::new(header).array().unwrap(), 0x0102);
    expect_error(Cursor::new(&header[..2]).array(), Errno::EPROTO);

    let array = b"\x04\x05\0list\0\x0b\x01\0\x05\x02\0x\0\x0c";
    let mut cursor = Cursor::new(array);
    let range = cursor
        .optional_array(b"list", ArrayLayout::Bare, |cursor| {
            assert_eq!(cursor.array()?, 1);
            assert_eq!(cursor.string()?, b"x");
            Ok(())
        })
        .unwrap();
    assert_eq!(range, Some(0..array.len()));
    assert!(cursor.is_eof());

    let mut incomplete = Cursor::new(&array[..array.len() - 1]);
    expect_error(
        incomplete.optional_array(b"list", ArrayLayout::Bare, |cursor| {
            cursor.array()?;
            cursor.string()?;
            Ok(())
        }),
        Errno::EPROTO,
    );
}

fn blob() {
    let bytes = b"\x04\x06\0aadfa\0\x06\x0a\0\0\0\x01\x02ab\0cdefg";
    let mut cursor = Cursor::new(bytes);
    assert_eq!(&bytes[cursor.dfa_blob().unwrap()], b"ab\0cdefg");
    assert!(cursor.is_eof());
    expect_error(Cursor::new(&bytes[..13]).dfa_blob(), Errno::EPROTO);
    expect_error(Cursor::new(&bytes[..23]).dfa_blob(), Errno::EPROTO);
}

fn profile() {
    let mut bytes = profile_fields();
    bytes.push(Code::StructEnd as u8);
    let record = decode_record(&bytes).unwrap();
    assert_eq!(record.name, b"test");
    assert_eq!(record.mode, ProfileMode::Complain);
    assert_eq!(record.capabilities.allow, 0x0000_0002_0004_0000);
    assert_eq!(record.capabilities.quiet, 0x0000_0004_0000_0000);
    expect_error(decode_record(&bytes[..bytes.len() - 1]), Errno::EPROTO);
}

fn profile_rlimits() {
    let mut bytes = profile_fields();
    let start = bytes.len();
    bytes.extend_from_slice(b"\x04\x08\0rlimits\0\x07");
    push_u32(&mut bytes, 1);
    bytes.push(Code::Array as u8);
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(Code::U64 as u8);
    bytes.extend_from_slice(&1024u64.to_le_bytes());
    let truncated = bytes.len() - 1;
    bytes.extend_from_slice(&[Code::ArrayEnd as u8, Code::StructEnd as u8]);
    let end = bytes.len();
    bytes.push(Code::StructEnd as u8);

    assert_eq!(decode_record(&bytes).unwrap().rlimits, Some(start..end));
    expect_error(decode_record(&bytes[..truncated]), Errno::EPROTO);
}

fn profile_fields() -> alloc::vec::Vec<u8> {
    let mut bytes =
        b"\x04\x08\0version\0\x02\x05\0\0\0\x04\x08\0profile\0\x07\x05\x05\0test\0".to_vec();
    bytes.extend_from_slice(b"\x04\x06\0flags\0\x07");
    for word in [0, 1, 0] {
        push_u32(&mut bytes, word);
    }
    bytes.push(Code::StructEnd as u8);
    for word in [0x0004_0000, 0, 0, 0] {
        push_u32(&mut bytes, word);
    }
    bytes.extend_from_slice(b"\x04\x07\0caps64\0\x07");
    for word in [2, 0, 4, 0] {
        push_u32(&mut bytes, word);
    }
    bytes.push(Code::StructEnd as u8);
    bytes
}

fn push_u32(bytes: &mut alloc::vec::Vec<u8>, value: u32) {
    bytes.push(Code::U32 as u8);
    bytes.extend_from_slice(&value.to_le_bytes());
}
