// SPDX-License-Identifier: MPL-2.0

//! Boot initramfs unpacking and init selection.
//!
//! This module unpacks the boot initramfs into the bootstrap VFS root and selects the initramfs
//! init from the `rdinit` parameter or the default `/init` path.

// Set this module's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "initramfs: "
    };
}

use alloc::{
    borrow::Cow,
    io::{self, Cursor, Read},
};

use cpio_decoder::{CpioDecoder, CpioEntry, FileMetadata, FileType};
use device_id::{DeviceId, MajorId, MinorId};
use lending_iterator::LendingIterator;
use ostd::boot::boot_info;
use spin::once::Once;
use zune_inflate::DeflateDecoder;

use super::{
    file::{InodeMode, InodeType},
    vfs::path::{FsPath, Path, PathResolver, is_dot},
};
use crate::{
    fs::{
        file::StatusFlags,
        vfs::inode::{Inode, MknodType},
    },
    prelude::*,
};

/// Unpacks the boot initramfs into the bootstrap root filesystem.
///
/// Returns successfully without changing the filesystem when no initramfs was supplied.
pub(crate) fn init_in_first_kthread(path_resolver: &PathResolver) -> Result<()> {
    let Some(initramfs_buf) = boot_info().initramfs else {
        return Ok(());
    };

    let (reader, suffix) = match &initramfs_buf[..4] {
        // Gzip magic number: 0x1F 0x8B
        &[0x1F, 0x8B, _, _] => {
            let decompressed = DeflateDecoder::new(initramfs_buf)
                .decode_gzip()
                .map_err(|_| Error::with_message(Errno::EINVAL, "gzip decompression failed"))?;
            (Cow::Owned(decompressed), ".gz")
        }
        _ => (Cow::Borrowed(initramfs_buf), ""),
    };

    println!("[kernel] unpacking initramfs.cpio{} to rootfs ...", suffix);

    let mut decoder = CpioDecoder::new(Cursor::new(reader));

    while let Some(entry_result) = decoder.next() {
        let mut entry = entry_result?;
        if let Err(e) = try_append_entry_to_rootfs(&mut entry, path_resolver) {
            warn!("failed to add entry {} to rootfs: {:?}", entry.name(), e);
        }
    }

    println!("[kernel] initramfs is ready");
    Ok(())
}

/// Finds the init program to run from the initramfs.
///
/// Resolves the path specified by `rdinit`, or `/init` when `rdinit` is not provided, and returns
/// the resolved path together with the original pathname. Returns an error if the pathname is
/// invalid or cannot be resolved.
pub(crate) fn find_init(path_resolver: &PathResolver) -> Result<(Path, &'static str)> {
    const DEFAULT_INITRAMFS_INIT_PATH: &str = "/init";

    let init_path = RDINIT_PATH
        .get()
        .map(String::as_str)
        .unwrap_or(DEFAULT_INITRAMFS_INIT_PATH);
    let path = path_resolver.lookup(&FsPath::try_from(init_path)?)?;
    Ok((path, init_path))
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
            let writer = InodeWriter {
                inner: path.inode().as_ref(),
                offset: 0,
            };
            entry.read_all(writer)?;
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

struct InodeWriter<'a> {
    inner: &'a dyn Inode,
    offset: usize,
}

impl io::Write for InodeWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        let write_len = self
            .inner
            .write_at(self.offset, &mut reader, StatusFlags::empty())
            .map_err(|_| io::ErrorKind::WriteZero)?;
        self.offset += write_len;
        Ok(write_len)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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

static RDINIT_PATH: Once<String> = Once::new();
aster_cmdline::define_kv_param!("rdinit", RDINIT_PATH);
