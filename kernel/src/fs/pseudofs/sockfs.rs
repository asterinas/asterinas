// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use spin::Once;

use crate::{
    fs::{
        path::{Mount, Path},
        pseudofs::{PseudoFs, PseudoInodeType},
        registry::{FsProperties, FsType},
        utils::{FileSystem, FsFlags, mkmod},
    },
    prelude::*,
    process::{Gid, Uid},
};

pub struct SockFs {
    _private: (),
}

impl SockFs {
    /// Returns the singleton instance of the socket file system.
    pub fn singleton() -> &'static Arc<PseudoFs> {
        static SOCKFS: Once<Arc<PseudoFs>> = Once::new();

        PseudoFs::singleton(&SOCKFS, "sockfs", SOCKFS_MAGIC)
    }

    /// Creates a pseudo `Path` for a socket.
    pub fn new_path() -> Path {
        let socket_inode = Arc::new(Self::singleton().alloc_inode(
            PseudoInodeType::Socket,
            mkmod!(a+rwx),
            Uid::new_root(),
            Gid::new_root(),
        ));

        Path::new_pseudo(Self::mount_node().clone(), socket_inode, |inode| {
            format!("socket:[{}]", inode.ino())
        })
    }

    /// Returns the pseudo mount node of the socket file system.
    pub fn mount_node() -> &'static Arc<Mount> {
        static SOCKFS_MOUNT: Once<Arc<Mount>> = Once::new();

        SOCKFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()))
    }
}

pub(super) struct SockFsType;

impl FsType for SockFsType {
    fn name(&self) -> &'static str {
        "sockfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _flags: FsFlags,
        _args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        return_errno_with_message!(Errno::EINVAL, "sockfs cannot be mounted");
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/magic.h#L89>
const SOCKFS_MAGIC: u64 = 0x534F434B;
