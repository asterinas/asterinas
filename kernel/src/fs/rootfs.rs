// SPDX-License-Identifier: MPL-2.0

use cpio_decoder::{CpioDecoder, FileType};
use lending_iterator::LendingIterator;
use libflate::gzip::Decoder as GZipDecoder;
use spin::Once;

use super::{
    fs_resolver::{FsPath, FsResolver},
    path::MountNode,
    procfs::{self, ProcFS},
    ramfs::RamFS,
    utils::{FileSystem, InodeMode, InodeType},
};
use crate::prelude::*;

/// Unpack and prepare the rootfs from the initramfs CPIO buffer.
pub fn init(initramfs_buf: &[u8]) -> Result<()> {
    init_root_mount();
    procfs::init();

    println!("[kernel] unpacking the initramfs.cpio.gz to rootfs ...");
    let fs = FsResolver::new();
    let mut decoder = CpioDecoder::new(
        GZipDecoder::new(initramfs_buf)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid gzip buffer"))?,
    );

    loop {
        let Some(entry_result) = decoder.next() else {
            break;
        };

        let mut entry = entry_result?;

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
                let dentry = parent.new_fs_child(name, InodeType::File, mode)?;
                entry.read_all(dentry.inode().writer(0))?;
            }
            FileType::Dir => {
                let _ = parent.new_fs_child(name, InodeType::Dir, mode)?;
            }
            FileType::Link => {
                let dentry = parent.new_fs_child(name, InodeType::SymLink, mode)?;
                let link_content = {
                    let mut link_data: Vec<u8> = Vec::new();
                    entry.read_all(&mut link_data)?;
                    core::str::from_utf8(&link_data)?.to_string()
                };
                dentry.inode().write_link(&link_content)?;
            }
            type_ => {
                panic!("unsupported file type = {:?} in initramfs", type_);
            }
        }
    }
    // Mount ProcFS
    let proc_dentry = fs.lookup(&FsPath::try_from("/proc")?)?;
    proc_dentry.mount(ProcFS::new())?;
    // Mount DevFS
    let dev_dentry = fs.lookup(&FsPath::try_from("/dev")?)?;
    dev_dentry.mount(RamFS::new())?;

    println!("[kernel] rootfs is ready");

    Ok(())
}

pub fn mount_fs_at(fs: Arc<dyn FileSystem>, fs_path: &FsPath) -> Result<()> {
    let target_dentry = FsResolver::new().lookup(fs_path)?;
    target_dentry.mount(fs)?;
    Ok(())
}

static ROOT_MOUNT: Once<Arc<MountNode>> = Once::new();

pub fn init_root_mount() {
    ROOT_MOUNT.call_once(|| -> Arc<MountNode> {
        let rootfs = RamFS::new();
        MountNode::new_root(rootfs)
    });
}

pub fn root_mount() -> &'static Arc<MountNode> {
    ROOT_MOUNT.get().unwrap()
}
