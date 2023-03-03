use crate::prelude::*;

use super::fs_resolver::{FsPath, FsResolver};
use super::utils::{InodeMode, InodeType};
use cpio_decoder::{CpioDecoder, FileType};

/// Unpack and prepare the fs from the ramdisk CPIO buffer.
pub fn init(ramdisk_buf: &[u8]) -> Result<()> {
    let decoder = CpioDecoder::new(ramdisk_buf);
    let fs = FsResolver::new();
    for entry_result in decoder.decode_entries() {
        let entry = entry_result?;

        // Make sure the name is a relative path, and is not end with "/".
        let entry_name = entry.name().trim_start_matches('/').trim_end_matches('/');
        if entry_name.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "invalid entry name");
        }
        if entry_name == "." {
            continue;
        }

        // Here we assume that the directory referred by "prefix" must has been created.
        // The basis of this assumption is：
        // The mkinitramfs script uses `find` command to ensure that the entries are
        // sorted that a directory always appears before its child directories and files.
        let (parent, name) = if let Some((prefix, last)) = entry_name.rsplit_once('/') {
            (fs.lookup(&FsPath::try_from(prefix)?)?, last)
        } else {
            (fs.root().clone(), entry_name)
        };

        let metadata = entry.metadata();
        let mode = InodeMode::from_bits_truncate(metadata.permission_mode());
        match metadata.file_type() {
            FileType::File => {
                let dentry = parent.create(name, InodeType::File, mode)?;
                dentry.vnode().write_at(0, entry.data())?;
            }
            FileType::Dir => {
                let _ = parent.create(name, InodeType::Dir, mode)?;
            }
            FileType::Link => {
                let dentry = parent.create(name, InodeType::SymLink, mode)?;
                let link_content = core::str::from_utf8(entry.data())?;
                dentry.vnode().write_link(link_content)?;
            }
            type_ => {
                warn!("unsupported file type = {:?} in initramfs", type_);
            }
        }
    }

    Ok(())
}
