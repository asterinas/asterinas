// SPDX-License-Identifier: MPL-2.0

//! Root filesystem bootstrapping and mounting.
//!
//! This module unpacks the boot initramfs into the bootstrap VFS root.
//! It also mounts a supported block-backed root filesystem in a new [`MountNamespace`].
//! [`RootFsType`] identifies the supported filesystem candidates.
//! [`mount`] resolves the selected block device and constructs the namespace.
//!
//! The implementation builds on the VFS [`FileSystem`], [`Mount`], and [`PathResolver`] abstractions.
//! It obtains block devices from `aster_block`.

use alloc::borrow::Cow;
use core::str::FromStr;

use cpio_decoder::{CpioDecoder, CpioEntry, FileMetadata, FileType};
use device_id::{DeviceId, MajorId, MinorId};
use lending_iterator::LendingIterator;
use no_std_io2::io::{Cursor, Read};
use ostd::boot::boot_info;
use zune_inflate::DeflateDecoder;

use super::{
    ext2::Ext2,
    file::{InodeMode, InodeType},
    vfs::{
        file_system::FileSystem,
        path::{FsPath, Mount, MountNamespace, PathResolver, PerMountFlags, is_dot},
    },
};
use crate::{fs::vfs::inode::MknodType, prelude::*, process::UserNamespace};

macro_rules! define_rootfs_types {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// A root filesystem type candidate.
        pub enum RootFsType {
            $($variant),+
        }

        impl RootFsType {
            /// Contains all supported root filesystem types.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
        }

        impl FromStr for RootFsType {
            type Err = core::convert::Infallible;

            fn from_str(type_name: &str) -> Result<Self, Self::Err> {
                match type_name {
                    $($name => Ok(Self::$variant),)+
                    _ => panic!("unsupported root filesystem type '{}'", type_name),
                }
            }
        }
    };
}

define_rootfs_types! {
    Ext2 => "ext2",
}

/// Unpacks the boot initramfs into the bootstrap root filesystem.
///
/// Returns successfully without changing the filesystem when no initramfs was supplied.
pub fn init_in_first_kthread(path_resolver: &PathResolver) -> Result<()> {
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

    println!("[kernel] rootfs is ready");
    Ok(())
}

/// Mounts the specified block device as a root filesystem in a new mount namespace.
pub fn mount(
    root: &str,
    rootfs_types: &[RootFsType],
    mount_flags: PerMountFlags,
) -> Result<Arc<MountNamespace>> {
    // Treat `root=/dev/...` as a Linux-compatible root device spec, not as a
    // VFS path lookup. Linux also mounts the root filesystem before
    // auto-mounting devtmpfs on `/dev`.
    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/base/devtmpfs.c#L358-L359>.
    let device_name = root
        .strip_prefix("/dev/")
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "root must name a /dev block device"))?;
    let device = aster_block::lookup_by_name(device_name)
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "root block device not found"))?;
    let fs = open_rootfs_from_candidates(device, rootfs_types)?;

    let owner = UserNamespace::get_init_singleton().clone();
    let mount_namespace = MountNamespace::new_with_root(owner, |weak_ns| {
        Mount::new_root_with_flags(fs, mount_flags, weak_ns.clone())
    })?;
    println!("[kernel] mounted {} as the root filesystem", root);
    Ok(mount_namespace)
}

fn open_rootfs_from_candidates(
    device: Arc<dyn aster_block::BlockDevice>,
    rootfs_types: &[RootFsType],
) -> Result<Arc<dyn FileSystem>> {
    for rootfs_type in rootfs_types {
        let result = match rootfs_type {
            RootFsType::Ext2 => {
                Ext2::open(device.clone(), None).map(|fs| fs as Arc<dyn FileSystem>)
            }
        };

        match result {
            Ok(fs) => return Ok(fs),
            Err(err) if err.error() == Errno::EINVAL => continue,
            Err(err) => return Err(err),
        }
    }

    return_errno_with_message!(Errno::ENODEV, "no root filesystem type could mount root")
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
