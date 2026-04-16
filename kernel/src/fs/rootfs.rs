// SPDX-License-Identifier: MPL-2.0

use cpio_decoder::{CpioDecoder, CpioEntry, FileMetadata, FileType};
use device_id::{DeviceId, MajorId, MinorId};
use lending_iterator::LendingIterator;
use libflate::gzip::Decoder as GZipDecoder;
use no_std_io2::io::{Cursor, Read};
use ostd::boot::boot_info;

use super::{
    file::{InodeMode, InodeType},
    vfs::path::{FsPath, PathResolver, is_dot},
};
use crate::{fs::vfs::inode::MknodType, prelude::*};

struct BoxedReader<'a>(Box<dyn Read + 'a>);

impl<'a> BoxedReader<'a> {
    pub fn new(reader: Box<dyn Read + 'a>) -> Self {
        BoxedReader(reader)
    }
}

impl Read for BoxedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> no_std_io2::io::Result<usize> {
        self.0.read(buf)
    }
}

/// Unpack and prepare the rootfs from the initramfs CPIO buffer.
pub fn init_in_first_kthread(path_resolver: &PathResolver) -> Result<()> {
    let initramfs_buf = boot_info()
        .initramfs
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "no initramfs found"))?;

    let (reader, suffix) = match &initramfs_buf[..4] {
        // Gzip magic number: 0x1F 0x8B
        &[0x1F, 0x8B, _, _] => {
            let gzip_decoder = GZipDecoder::new(initramfs_buf)
                .map_err(|_| Error::with_message(Errno::EINVAL, "invalid gzip buffer"))?;
            (BoxedReader::new(Box::new(gzip_decoder)), ".gz")
        }
        _ => (BoxedReader::new(Box::new(Cursor::new(initramfs_buf))), ""),
    };

    println!("[kernel] unpacking initramfs.cpio{} to rootfs ...", suffix);

    let mut decoder = CpioDecoder::new(reader);

    while let Some(entry_result) = decoder.next() {
        let mut entry = entry_result?;
        if let Err(e) = try_append_entry_to_rootfs(&mut entry, path_resolver) {
            warn!("failed to add entry {} to rootfs: {:?}", entry.name(), e);
        }
    }

    println!("[kernel] rootfs is ready");
    Ok(())
}

fn try_append_entry_to_rootfs(
    entry: &mut CpioEntry<BoxedReader>,
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
