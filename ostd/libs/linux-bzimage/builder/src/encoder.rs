// SPDX-License-Identifier: MPL-2.0

//! This module is used to compress kernel ELF.

use std::{
    ffi::{OsStr, OsString},
    str::FromStr,
};

use miniz_oxide::deflate::{compress_to_vec, compress_to_vec_zlib};
use serde::{Deserialize, Serialize};

/// The DEFLATE compression level (0-10) used for the payload. 6 matches the
/// common default and balances ratio against build time.
const COMPRESSION_LEVEL: u8 = 6;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum PayloadEncoding {
    #[default]
    #[serde(rename = "raw")]
    Raw,
    #[serde(rename = "gzip")]
    Gzip,
    #[serde(rename = "zlib")]
    Zlib,
}

impl FromStr for PayloadEncoding {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "raw" => Ok(Self::Raw),
            "gzip" => Ok(Self::Gzip),
            "zlib" => Ok(Self::Zlib),
            _ => Err(format!("Invalid encoding format: {}", s)),
        }
    }
}

impl From<OsString> for PayloadEncoding {
    fn from(os_string: OsString) -> Self {
        PayloadEncoding::from_str(&os_string.to_string_lossy()).unwrap()
    }
}

impl From<&OsStr> for PayloadEncoding {
    fn from(os_str: &OsStr) -> Self {
        PayloadEncoding::from_str(&os_str.to_string_lossy()).unwrap()
    }
}

/// Encoding the kernel ELF using the provided format.
pub fn encode_kernel(kernel: Vec<u8>, encoding: PayloadEncoding) -> Vec<u8> {
    match encoding {
        PayloadEncoding::Raw => kernel,
        // `miniz_oxide` does not produce the gzip wrapper, so build it manually:
        // the fixed 10-byte header, the raw DEFLATE body, then the CRC32 and the
        // uncompressed size (mod 2^32) as the footer.
        //
        // Reference: <https://datatracker.ietf.org/doc/html/rfc1952>.
        PayloadEncoding::Gzip => {
            let mut out = vec![
                0x1F, 0x8B, // Magic.
                0x08, // CM: DEFLATE.
                0x00, // FLG: no optional fields.
                0x00, 0x00, 0x00, 0x00, // MTIME: none.
                0x00, // XFL.
                0xFF, // OS: unknown.
            ];
            out.extend_from_slice(&compress_to_vec(&kernel, COMPRESSION_LEVEL));
            out.extend_from_slice(&crc32fast::hash(&kernel).to_le_bytes());
            out.extend_from_slice(&(kernel.len() as u32).to_le_bytes());
            out
        }
        PayloadEncoding::Zlib => compress_to_vec_zlib(&kernel, COMPRESSION_LEVEL),
    }
}
