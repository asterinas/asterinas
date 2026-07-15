// SPDX-License-Identifier: MPL-2.0

//! This module is used to decompress payload.

extern crate alloc;

use alloc::vec::Vec;
use core::convert::TryFrom;

use miniz_oxide::inflate::{decompress_to_vec, decompress_to_vec_zlib};

enum MagicNumber {
    Elf,
    Gzip,
    Zlib,
}

#[derive(Debug)]
struct InvalidMagicNumber;

impl TryFrom<&[u8]> for MagicNumber {
    type Error = InvalidMagicNumber;

    fn try_from(slice: &[u8]) -> Result<Self, Self::Error> {
        match *slice {
            [0x7F, 0x45, 0x4C, 0x46, ..] => Ok(Self::Elf),
            [0x1F, 0x8B, ..] => Ok(Self::Gzip),
            [0x78, 0x9C, ..] => Ok(Self::Zlib),
            _ => Err(InvalidMagicNumber),
        }
    }
}

/// Detects the format used to encode the payload and decodes the payload accordingly.
pub fn decode_payload(payload: &[u8]) -> Vec<u8> {
    let magic = MagicNumber::try_from(payload).unwrap();
    match magic {
        MagicNumber::Elf => payload.to_vec(),
        // `miniz_oxide` does not parse the gzip wrapper, so strip the header and
        // inflate the raw DEFLATE body (the trailing footer is ignored, as
        // inflation stops at the end of the DEFLATE stream).
        MagicNumber::Gzip => decompress_to_vec(strip_gzip_header(payload)).unwrap(),
        MagicNumber::Zlib => decompress_to_vec_zlib(payload).unwrap(),
    }
}

// The gzip header flag bits.
// Reference: <https://datatracker.ietf.org/doc/html/rfc1952>.
const FLG_FHCRC: u8 = 0x02;
const FLG_FEXTRA: u8 = 0x04;
const FLG_FNAME: u8 = 0x08;
const FLG_FCOMMENT: u8 = 0x10;

/// Parses the gzip header and returns the remaining bytes.
fn strip_gzip_header(buf: &[u8]) -> &[u8] {
    // Fixed 10-byte header: ID1, ID2, CM, FLG, MTIME(4), XFL, OS.
    let flg = buf[3];
    let mut pos = 10;

    if flg & FLG_FEXTRA != 0 {
        let xlen = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
        pos += 2 + xlen;
    }
    if flg & FLG_FNAME != 0 {
        pos += buf[pos..].iter().position(|&b| b == 0).unwrap() + 1;
    }
    if flg & FLG_FCOMMENT != 0 {
        pos += buf[pos..].iter().position(|&b| b == 0).unwrap() + 1;
    }
    if flg & FLG_FHCRC != 0 {
        pos += 2;
    }

    &buf[pos..]
}
