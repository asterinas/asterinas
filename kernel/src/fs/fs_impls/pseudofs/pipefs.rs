// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use spin::Once;

use crate::{
    fs::{
        path::{Mount, Path},
        pipe::AnonPipeInode,
        pseudofs::PseudoFs,
        registry::{FsProperties, FsType},
        utils::{FileSystem, FsFlags},
    },
    prelude::*,
};

pub(in crate::fs) struct PipeFs {
    _private: (),
}

impl PipeFs {
    /// Returns the singleton instance of the anonymous pipe file system.
    pub(in crate::fs) fn singleton() -> &'static Arc<PseudoFs> {
        static PIPEFS: Once<Arc<PseudoFs>> = Once::new();

        PseudoFs::singleton(&PIPEFS, "pipefs", PIPEFS_MAGIC)
    }

    /// Creates a pseudo `Path` for an anonymous pipe.
    pub(in crate::fs) fn new_path(pipe_inode: Arc<AnonPipeInode>) -> Path {
        Path::new_pseudo(Self::mount_node().clone(), pipe_inode, |inode| {
            format!("pipe:[{}]", inode.ino())
        })
    }

    /// Returns the pseudo mount node of the pipe file system.
    fn mount_node() -> &'static Arc<Mount> {
        static PIPEFS_MOUNT: Once<Arc<Mount>> = Once::new();

        PIPEFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()))
    }
}

pub(super) struct PipeFsType;

impl FsType for PipeFsType {
    fn name(&self) -> &'static str {
        "pipefs"
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
        return_errno_with_message!(Errno::EINVAL, "pipefs cannot be mounted");
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/magic.h#L87>
const PIPEFS_MAGIC: u64 = 0x50495045;
