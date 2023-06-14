//! A safe Rust CPIO (the newc format) decoder.
//!
//! # Example
//!
//! ```rust
//! use cpio_decoder::CpioDecoder;
//!
//! let decoder = CpioDecoder::new(&[]);
//! for entry_result in decoder.decode_entries() {
//!     println!("The entry_result is: {:?}", entry_result);
//! }
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use crate::error::{Error, Result};
use int_to_c_enum::TryFromInt;

pub mod error;

#[cfg(test)]
mod test;

/// A CPIO (the newc format) decoder.
///
/// "newc" is the new portable format and CRC format.
///
/// Each file has a 110 byte header, a variable length NULL-terminated filename,
/// and variable length file data.
/// A header for a filename "TRAILER!!!" indicates the end of the archive.
///
/// All the fields in the header are ISO 646 (approximately ASCII) strings
/// of hexadecimal numbers, left padded, not NULL terminated.
pub struct CpioDecoder<'a> {
    buffer: &'a [u8],
}

impl<'a> CpioDecoder<'a> {
    /// create a decoder to decode the CPIO.
    pub fn new(buffer: &'a [u8]) -> Self {
        Self { buffer }
    }

    /// Return an iterator trying to decode the entries in the CPIO.
    pub fn decode_entries(&'a self) -> CpioEntryIter<'a> {
        CpioEntryIter::new(self)
    }
}

/// An iterator over the results of CPIO entries.
///
/// It stops if reaches to the trailer entry or encounters an error.
pub struct CpioEntryIter<'a> {
    buffer: &'a [u8],
    offset: usize,
    is_error: bool,
}

impl<'a> CpioEntryIter<'a> {
    fn new(decoder: &'a CpioDecoder) -> Self {
        Self {
            buffer: decoder.buffer,
            offset: 0,
            is_error: false,
        }
    }
}

impl<'a> Iterator for CpioEntryIter<'a> {
    type Item = Result<CpioEntry<'a>>;

    fn next(&mut self) -> Option<Result<CpioEntry<'a>>> {
        // Stop to iterate entries if encounters an error.
        if self.is_error {
            return None;
        }

        let entry_result = if self.offset >= self.buffer.len() {
            Err(Error::BufferShortError)
        } else {
            CpioEntry::new(&self.buffer[self.offset..])
        };
        match &entry_result {
            Ok(entry) => {
                // A correct CPIO buffer must end with a trailer.
                if entry.is_trailer() {
                    return None;
                }
                self.offset += entry.archive_offset();
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
pub struct CpioEntry<'a> {
    metadata: FileMetadata,
    name: &'a str,
    data: &'a [u8],
}

impl<'a> CpioEntry<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        let (metadata, name, data) = {
            let header = Header::new(bytes)?;
            let name = {
                let bytes_remain = &bytes[HEADER_LEN..];
                let name_size = read_hex_bytes_to_u32(header.name_size)? as usize;
                if bytes_remain.len() < name_size {
                    return Err(Error::BufferShortError);
                }
                let name = core::ffi::CStr::from_bytes_with_nul(&bytes_remain[..name_size])
                    .map_err(|_| Error::FileNameError)?;
                name.to_str().map_err(|_| Error::Utf8Error)?
            };
            let metadata = if name == TRAILER_NAME {
                Default::default()
            } else {
                FileMetadata::new(header)?
            };
            let data = {
                let data_size = metadata.size as usize;
                if data_size == 0 {
                    &[]
                } else {
                    let data_offset = align_up(HEADER_LEN + name.len() + 1, 4);
                    if data_offset + data_size > bytes.len() {
                        return Err(Error::BufferShortError);
                    }
                    &bytes[data_offset..data_offset + data_size]
                }
            };

            (metadata, name, data)
        };
        Ok(Self {
            metadata,
            name,
            data,
        })
    }

    /// The metadata of the file.
    pub fn metadata(&self) -> &FileMetadata {
        &self.metadata
    }

    /// The name of the file.
    pub fn name(&self) -> &str {
        self.name
    }

    /// The data of the file.
    pub fn data(&self) -> &[u8] {
        self.data
    }

    fn is_trailer(&self) -> bool {
        self.name == TRAILER_NAME
    }

    fn archive_offset(&self) -> usize {
        align_up(HEADER_LEN + self.name.len() + 1, 4) + align_up(self.data.len(), 4)
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
    fn new(header: Header) -> Result<Self> {
        const MODE_MASK: u32 = 0o7777;
        let raw_mode = read_hex_bytes_to_u32(&header.mode)?;
        let metadata = Self {
            ino: read_hex_bytes_to_u32(&header.ino)?,
            type_: FileType::try_from(raw_mode).map_err(|_| Error::FileTypeError)?,
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

const HEADER_LEN: usize = 110;
const MAGIC: &[u8] = b"070701";
const TRAILER_NAME: &str = "TRAILER!!!";

#[rustfmt::skip]
struct Header<'a> {
    // magic: &'a [u8],     // [u8; 6]
    ino: &'a [u8],          // [u8; 8]
    mode: &'a [u8],         // [u8; 8]
    uid: &'a [u8],          // [u8; 8]
    gid: &'a [u8],          // [u8; 8]
    nlink: &'a [u8],        // [u8; 8]
    mtime: &'a [u8],        // [u8; 8]
    file_size: &'a [u8],    // [u8; 8]
    dev_maj: &'a [u8],      // [u8; 8]
    dev_min: &'a [u8],      // [u8; 8]
    rdev_maj: &'a [u8],     // [u8; 8]
    rdev_min: &'a [u8],     // [u8; 8]
    name_size: &'a [u8],    // [u8; 8]
    // chksum: &'a [u8],    // [u8; 8]
}

impl<'a> Header<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferShortError);
        }
        let magic = &bytes[..6];
        if magic != MAGIC {
            return Err(Error::MagicError);
        }
        Ok(Self {
            ino: &bytes[6..14],
            mode: &bytes[14..22],
            uid: &bytes[22..30],
            gid: &bytes[30..38],
            nlink: &bytes[38..46],
            mtime: &bytes[46..54],
            file_size: &bytes[54..62],
            dev_maj: &bytes[62..70],
            dev_min: &bytes[70..78],
            rdev_maj: &bytes[78..86],
            rdev_min: &bytes[86..94],
            name_size: &bytes[94..102],
        })
    }
}

fn read_hex_bytes_to_u32(bytes: &[u8]) -> Result<u32> {
    debug_assert!(bytes.len() == 8);
    let string = core::str::from_utf8(bytes).map_err(|_| Error::Utf8Error)?;
    let num = u32::from_str_radix(string, 16).map_err(|_| Error::ParseIntError)?;
    Ok(num)
}

fn align_up(size: usize, align: usize) -> usize {
    debug_assert!(align >= 2 && align.is_power_of_two());
    (size + align - 1) & !(align - 1)
}
