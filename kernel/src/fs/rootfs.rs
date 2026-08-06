// SPDX-License-Identifier: MPL-2.0

use cpio_decoder::{CpioDecoder, CpioEntry, FileMetadata, FileType};
use device_id::{DeviceId, MajorId, MinorId};
use lending_iterator::LendingIterator;
use miniz_oxide::{
    DataFormat, MZFlush, MZStatus,
    inflate::stream::{InflateState, inflate},
};
use no_std_io2::io::{self, Cursor, Read};
use ostd::boot::boot_info;

use super::{
    file::{InodeMode, InodeType},
    vfs::path::{FsPath, PathResolver, is_dot},
};
use crate::{fs::vfs::inode::MknodType, prelude::*};

/// Unpack and prepare the rootfs from the initramfs CPIO buffer.
pub fn init_in_first_kthread(path_resolver: &PathResolver) -> Result<()> {
    let initramfs_buf = boot_info()
        .initramfs
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "no initramfs found"))?;

    match &initramfs_buf[..4] {
        // Gzip magic number: 0x1F 0x8B
        &[0x1F, 0x8B, _, _] => {
            println!("[kernel] unpacking initramfs.cpio.gz to rootfs ...");
            unpack_to_rootfs(
                CpioDecoder::new(GzipReader::new(initramfs_buf)),
                path_resolver,
            )?;
        }
        _ => {
            println!("[kernel] unpacking initramfs.cpio to rootfs ...");
            unpack_to_rootfs(CpioDecoder::new(Cursor::new(initramfs_buf)), path_resolver)?;
        }
    }

    println!("[kernel] rootfs is ready");
    Ok(())
}

/// Unpacks every entry of a CPIO archive into the rootfs.
fn unpack_to_rootfs<R: Read>(
    mut decoder: CpioDecoder<R>,
    path_resolver: &PathResolver,
) -> Result<()> {
    while let Some(entry_result) = decoder.next() {
        let mut entry = entry_result?;
        if let Err(e) = try_append_entry_to_rootfs(&mut entry, path_resolver) {
            warn!("failed to add entry {} to rootfs: {:?}", entry.name(), e);
        }
    }

    Ok(())
}

fn try_append_entry_to_rootfs<R: Read>(
    entry: &mut CpioEntry<R>,
    path_resolver: &PathResolver,
) -> Result<()> {
    // Make sure the name is a relative path, and is not end with "/".
    let entry_name = entry.name().trim_start_matches('/').trim_end_matches('/');
    if entry_name.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "invalid entry name");
    }
    if is_dot(entry_name) {
        return Ok(());
    }

    // Here we assume that the directory referred by "prefix" must has been created.
    // The basis of this assumption is：
    // The mkinitramfs script uses `find` command to ensure that the entries are
    // sorted that a directory always appears before its child directories and files.
    let (parent, name) = if let Some((prefix, last)) = entry_name.rsplit_once('/') {
        (path_resolver.lookup(&FsPath::try_from(prefix)?)?, last)
    } else {
        (path_resolver.root().clone(), entry_name)
    };

    let metadata = entry.metadata();
    let mode = InodeMode::from_bits_truncate(metadata.permission_mode());
    match metadata.file_type() {
        FileType::File => {
            let path = parent.new_fs_child(name, InodeType::File, mode)?;
            entry.read_all(path.inode().writer(0))?;
        }
        FileType::Dir => {
            let _ = parent.new_fs_child(name, InodeType::Dir, mode)?;
        }
        FileType::Link => {
            let path = parent.new_fs_child(name, InodeType::SymLink, mode)?;
            let link_content = {
                let mut link_data: Vec<u8> = Vec::new();
                entry.read_all(&mut link_data)?;
                core::str::from_utf8(&link_data)?.to_string()
            };
            path.inode().write_link(&link_content)?;
        }
        FileType::Char => {
            let device_id = try_device_id_from_metadata(metadata)?;
            parent.mknod(name, mode, MknodType::CharDevice(device_id))?;
        }
        FileType::Block => {
            let device_id = try_device_id_from_metadata(metadata)?;
            parent.mknod(name, mode, MknodType::BlockDevice(device_id))?;
        }
        FileType::FiFo => {
            parent.mknod(name, mode, MknodType::NamedPipe)?;
        }
        FileType::Socket => {
            return_errno_with_message!(Errno::EINVAL, "socket files are not supported in initramfs")
        }
    }

    Ok(())
}

fn try_device_id_from_metadata(metadata: &FileMetadata) -> Result<u64> {
    let major = {
        let dev_maj = u16::try_from(metadata.rdev_maj())?;
        MajorId::try_from(dev_maj).map_err(|msg| Error::with_message(Errno::EINVAL, msg))?
    };
    let minor = MinorId::try_from(metadata.rdev_min())
        .map_err(|msg| Error::with_message(Errno::EINVAL, msg))?;
    Ok(DeviceId::new(major, minor).as_encoded_u64())
}

/// A streaming gzip decompressor over an in-memory compressed buffer.
struct GzipReader<'a> {
    // The DEFLATE body not yet consumed, shrinking as it is read.
    deflate_body: &'a [u8],
    state: Box<InflateState>,
    done: bool,
}

impl<'a> GzipReader<'a> {
    /// Creates a decompressor, parsing and skipping the gzip header of `buf`.
    fn new(buf: &'a [u8]) -> Self {
        Self {
            deflate_body: strip_gzip_header(buf),
            state: InflateState::new_boxed(DataFormat::Raw),
            done: false,
        }
    }
}

impl Read for GzipReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }

        while !self.done {
            let result = inflate(&mut self.state, self.deflate_body, out, MZFlush::None);
            self.deflate_body = &self.deflate_body[result.bytes_consumed..];
            match result.status {
                Ok(MZStatus::StreamEnd) => self.done = true,
                Ok(_) => {}
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "gzip decompression failed",
                    ));
                }
            }
            if result.bytes_written > 0 {
                return Ok(result.bytes_written);
            }
            // No output and no input consumed means no further progress is
            // possible (truncated stream); stop to avoid spinning forever.
            if result.bytes_consumed == 0 {
                break;
            }
        }

        Ok(0)
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
