// SPDX-License-Identifier: MPL-2.0

//! This module is used to decompress payload.

extern crate alloc;

use alloc::vec::Vec;
use core::convert::TryFrom;

use core2::io::Read;
use libflate::{gzip, zlib};

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
    let mut kernel = Vec::new();
    let magic = MagicNumber::try_from(payload).unwrap();
    match magic {
        MagicNumber::Elf => {
            kernel = payload.to_vec();
        }
        MagicNumber::Gzip => {
            let mut decoder = gzip::Decoder::new(payload).unwrap();
            decoder.read_to_end(&mut kernel).unwrap();
        }
        MagicNumber::Zlib => {
            let mut decoder = zlib::Decoder::new(payload).unwrap();
            decoder.read_to_end(&mut kernel).unwrap();
        }
    }
    kernel
}
