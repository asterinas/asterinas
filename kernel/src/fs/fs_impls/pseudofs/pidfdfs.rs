// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{
    fs::{
        path::{Mount, Path},
        pseudofs::{PseudoFs, PseudoInodeType},
        utils::{Inode, mkmod},
    },
    prelude::*,
    process::{Gid, Uid},
};

pub struct PidfdFs {
    _private: (),
}

impl PidfdFs {
    /// Returns the singleton instance of the pidfd file system.
    pub fn singleton() -> &'static Arc<PseudoFs> {
        static PIDFDFS: Once<Arc<PseudoFs>> = Once::new();

        PseudoFs::singleton(&PIDFDFS, "pidfdfs", PIDFDFS_MAGIC)
    }

    /// Creates a pseudo `Path` for a pidfd.
    pub fn new_path(name_fn: fn(&dyn Inode) -> String) -> Path {
        Path::new_pseudo(
            Self::mount_node().clone(),
            Self::shared_inode().clone(),
            name_fn,
        )
    }

    /// Returns the pseudo mount node of the pidfd file system.
    pub fn mount_node() -> &'static Arc<Mount> {
        static PIDFDFS_MOUNT: Once<Arc<Mount>> = Once::new();

        PIDFDFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()))
    }

    /// Returns the shared inode of the pidfd file system.
    pub fn shared_inode() -> &'static Arc<dyn Inode> {
        static SHARED_INODE: Once<Arc<dyn Inode>> = Once::new();

        SHARED_INODE.call_once(|| {
            let pidfd_inode = Self::singleton().alloc_inode(
                PseudoInodeType::Pidfd,
                mkmod!(u+rwx),
                Uid::new_root(),
                Gid::new_root(),
            );

            Arc::new(pidfd_inode)
        })
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/magic.h#L105>
const PIDFDFS_MAGIC: u64 = 0x50494446;
