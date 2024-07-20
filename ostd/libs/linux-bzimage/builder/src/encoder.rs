// SPDX-License-Identifier: MPL-2.0

//! This module is used to compress kernel ELF.

use std::{
    ffi::{OsStr, OsString},
    io::Write,
    str::FromStr,
};

use libflate::{gzip, zlib};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        PayloadEncoding::Gzip => {
            let mut encoder = gzip::Encoder::new(Vec::new()).unwrap();
            encoder.write_all(&kernel).unwrap();
            encoder.finish().into_result().unwrap()
        }
        PayloadEncoding::Zlib => {
            let mut encoder = zlib::Encoder::new(Vec::new()).unwrap();
            encoder.write_all(&kernel).unwrap();
            encoder.finish().into_result().unwrap()
        }
    }
}
