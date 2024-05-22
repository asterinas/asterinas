// SPDX-License-Identifier: MPL-2.0

//! A safe Rust CPIO (the newc format) decoder.
//!
//! # Example
//!
//! ```rust
//! use cpio_decoder::CpioDecoder;
//! use lending_iterator::LendingIterator;
//!
//! let short_buffer: Vec<u8> = Vec::new();
//! let mut decoder = CpioDecoder::new(short_buffer.as_slice());
//! if let Some(entry_result) = decoder.next() {
//!     println!("The entry_result is: {:?}", entry_result);
//! }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]
#![allow(dead_code)]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec,
};
use core::cmp::min;

use core2::io::{Read, Write};
use int_to_c_enum::TryFromInt;
use lending_iterator::prelude::*;

use crate::error::{Error, Result};

pub mod error;

#[cfg(test)]
mod test;

/// A CPIO (the newc format) decoder to iterator over the results of CPIO entries.
///
/// "newc" is the new portable format and CRC format.
///
/// Each file has a 110 byte header, a variable length NULL-terminated filename,
/// and variable length file data.
/// A header for a filename "TRAILER!!!" indicates the end of the archive.
///
/// All the fields in the header are ISO 646 (approximately ASCII) strings
/// of hexadecimal numbers, left padded, not NULL terminated.
pub struct CpioDecoder<R> {
    reader: R,
    is_error: bool,
}

impl<R> CpioDecoder<R>
where
    R: Read,
{
    /// Create a decoder.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            is_error: false,
        }
    }
}

#[gat]
impl<R> LendingIterator for CpioDecoder<R>
where
    R: Read,
{
    type Item<'a> = Result<CpioEntry<'a, R>>;

    /// Stops if reaches to the trailer entry or encounters an error.
    fn next(&mut self) -> Option<Self::Item<'_>> {
        // Stop to iterate entries if encounters an error.
        if self.is_error {
            return None;
        }

        let entry_result = CpioEntry::new(&mut self.reader);
        match &entry_result {
            Ok(entry) => {
                // A correct CPIO buffer must end with a trailer.
                if entry.is_trailer() {
                    return None;
                }
            }
            Err(_) => {
                self.is_error = true;
            }
        }
        Some(entry_result)
    }
}

/// A file entry in the CPIO.
#[derive(Debug)]
pub struct CpioEntry<'a, R> {
    metadata: FileMetadata,
    name: String,
    reader: &'a mut R,
    data_padding_len: usize,
}

impl<'a, R> CpioEntry<'a, R>
where
    R: Read,
{
    fn new(reader: &'a mut R) -> Result<Self> {
        let (metadata, name, data_padding_len) = {
            let header = Header::new(reader)?;
            let name = {
                let name_size = read_hex_bytes_to_u32(&header.name_size)? as usize;
                let mut name_bytes = vec![0u8; name_size];
                reader.read_exact(&mut name_bytes)?;
                let name = core::ffi::CStr::from_bytes_with_nul(&name_bytes)
                    .map_err(|_| Error::FileNameError)?;
                name.to_str().map_err(|_| Error::Utf8Error)?.to_string()
            };
            let metadata = if name == TRAILER_NAME {
                Default::default()
            } else {
                FileMetadata::new(&header)?
            };
            let data_padding_len = {
                let header_padding_len = align_up_pad(header.len() + name.len() + 1, 4);
                if header_padding_len > 0 {
                    let mut pad_buf = vec![0u8; header_padding_len];
                    reader.read_exact(&mut pad_buf)?;
                }
                align_up_pad(metadata.size() as usize, 4)
            };

            (metadata, name, data_padding_len)
        };
        Ok(Self {
            metadata,
            name,
            reader,
            data_padding_len,
        })
    }

    /// The metadata of the file.
    pub fn metadata(&self) -> &FileMetadata {
        &self.metadata
    }

    /// The name of the file.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Read all data to the writer.
    pub fn read_all<W>(&mut self, mut writer: W) -> Result<()>
    where
        W: Write,
    {
        let data_len = self.metadata().size() as usize;
        let mut send_len = 0;
        let mut buffer = vec![0u8; 0x1000];
        while send_len < data_len {
            let len = min(buffer.len(), data_len - send_len);
            self.reader.read_exact(&mut buffer[..len])?;
            writer.write_all(&buffer[..len])?;
            send_len += len;
        }
        if self.data_padding_len > 0 {
            self.reader
                .read_exact(&mut buffer[..self.data_padding_len])?;
        }
        Ok(())
    }

    pub fn is_trailer(&self) -> bool {
        self.name == TRAILER_NAME
    }
}

/// The metadata of the file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileMetadata {
    ino: u32,
    type_: FileType,
    mode: u16,
    uid: u32,
    gid: u32,
    nlink: u32,
    mtime: u32,
    size: u32,
    dev_maj: u32,
    dev_min: u32,
    rdev_maj: u32,
    rdev_min: u32,
}

impl FileMetadata {
    fn new(header: &Header) -> Result<Self> {
        const MODE_MASK: u32 = 0o7777;
        const TYPE_MASK: u32 = 0o170000;
        let raw_mode = read_hex_bytes_to_u32(&header.mode)?;
        let metadata = Self {
            ino: read_hex_bytes_to_u32(&header.ino)?,
            type_: FileType::try_from(raw_mode & TYPE_MASK).map_err(|_| Error::FileTypeError)?,
            mode: (raw_mode & MODE_MASK) as u16,
            uid: read_hex_bytes_to_u32(&header.uid)?,
            gid: read_hex_bytes_to_u32(&header.gid)?,
            nlink: read_hex_bytes_to_u32(&header.nlink)?,
            mtime: read_hex_bytes_to_u32(&header.mtime)?,
            size: read_hex_bytes_to_u32(&header.file_size)?,
            dev_maj: read_hex_bytes_to_u32(&header.dev_maj)?,
            dev_min: read_hex_bytes_to_u32(&header.dev_min)?,
            rdev_maj: read_hex_bytes_to_u32(&header.rdev_maj)?,
            rdev_min: read_hex_bytes_to_u32(&header.rdev_min)?,
        };
        Ok(metadata)
    }

    /// The inode number.
    pub fn ino(&self) -> u32 {
        self.ino
    }

    /// The file type.
    pub fn file_type(&self) -> FileType {
        self.type_
    }

    /// The file permission mode, e.g., 0o0755.
    pub fn permission_mode(&self) -> u16 {
        self.mode
    }

    /// The user ID of the file owner.
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// The group ID of the file owner.
    pub fn gid(&self) -> u32 {
        self.gid
    }

    /// The number of hard links.
    pub fn nlink(&self) -> u32 {
        self.nlink
    }

    /// The last modification time.
    pub fn mtime(&self) -> u32 {
        self.mtime
    }

    /// The size of the file in bytes.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The device major ID on which the file resides.
    pub fn dev_maj(&self) -> u32 {
        self.dev_maj
    }

    /// The device minor ID on which the file resides.
    pub fn dev_min(&self) -> u32 {
        self.dev_min
    }

    /// The device major ID that the file represents. Only relevant for special file.
    pub fn rdev_maj(&self) -> u32 {
        self.rdev_maj
    }

    /// The device minor ID that the file represents. Only relevant for special file.
    pub fn rdev_min(&self) -> u32 {
        self.rdev_min
    }
}

/// The type of the file.
#[repr(u32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum FileType {
    /// FIFO special file
    FiFo = 0o010000,
    /// Character device
    Char = 0o020000,
    /// Directory
    Dir = 0o040000,
    /// Block device
    Block = 0o060000,
    /// Regular file
    File = 0o100000,
    /// Symbolic link
    Link = 0o120000,
    /// Socket
    Socket = 0o140000,
}

impl Default for FileType {
    fn default() -> Self {
        Self::File
    }
}

const MAGIC: &[u8] = b"070701";
const TRAILER_NAME: &str = "TRAILER!!!";

struct Header {
    magic: [u8; 6],
    ino: [u8; 8],
    mode: [u8; 8],
    uid: [u8; 8],
    gid: [u8; 8],
    nlink: [u8; 8],
    mtime: [u8; 8],
    file_size: [u8; 8],
    dev_maj: [u8; 8],
    dev_min: [u8; 8],
    rdev_maj: [u8; 8],
    rdev_min: [u8; 8],
    name_size: [u8; 8],
    chksum: [u8; 8],
}

impl Header {
    pub fn new<R>(reader: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let mut buf = vec![0u8; core::mem::size_of::<Self>()];
        reader.read_exact(&mut buf)?;

        let header = Self {
            magic: <[u8; 6]>::try_from(&buf[0..6]).unwrap(),
            ino: <[u8; 8]>::try_from(&buf[6..14]).unwrap(),
            mode: <[u8; 8]>::try_from(&buf[14..22]).unwrap(),
            uid: <[u8; 8]>::try_from(&buf[22..30]).unwrap(),
            gid: <[u8; 8]>::try_from(&buf[30..38]).unwrap(),
            nlink: <[u8; 8]>::try_from(&buf[38..46]).unwrap(),
            mtime: <[u8; 8]>::try_from(&buf[46..54]).unwrap(),
            file_size: <[u8; 8]>::try_from(&buf[54..62]).unwrap(),
            dev_maj: <[u8; 8]>::try_from(&buf[62..70]).unwrap(),
            dev_min: <[u8; 8]>::try_from(&buf[70..78]).unwrap(),
            rdev_maj: <[u8; 8]>::try_from(&buf[78..86]).unwrap(),
            rdev_min: <[u8; 8]>::try_from(&buf[86..94]).unwrap(),
            name_size: <[u8; 8]>::try_from(&buf[94..102]).unwrap(),
            chksum: <[u8; 8]>::try_from(&buf[102..110]).unwrap(),
        };
        if header.magic != MAGIC {
            return Err(Error::MagicError);
        }
        Ok(header)
    }

    fn len(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

fn read_hex_bytes_to_u32(bytes: &[u8]) -> Result<u32> {
    debug_assert!(bytes.len() == 8);
    let string = core::str::from_utf8(bytes).map_err(|_| Error::Utf8Error)?;
    let num = u32::from_str_radix(string, 16).map_err(|_| Error::ParseIntError)?;
    Ok(num)
}

fn align_up_pad(size: usize, align: usize) -> usize {
    align_up(size, align) - size
}

fn align_up(size: usize, align: usize) -> usize {
    debug_assert!(align >= 2 && align.is_power_of_two());
    (size + align - 1) & !(align - 1)
}
