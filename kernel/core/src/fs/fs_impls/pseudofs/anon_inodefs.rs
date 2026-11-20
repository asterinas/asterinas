// SPDX-License-Identifier: MPL-2.0

use core::fmt::{self, Display};

use spin::Once;

use crate::{
    fs::{
        file::{CreationFlags, FileCommon, file_table::FdFlags, mkmod},
        pseudofs::{NaivePseudoFs, PseudoInodeType},
        vfs::{
            inode::Inode,
            path::{Mount, Path},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

pub(super) fn init() {
    // Touch the mount node to trigger the initialization of all singletons.
    let _ = AnonInodeFs::mount_node();
}

pub(crate) struct AnonInodeFs {
    _private: (),
}

impl AnonInodeFs {
    /// Returns the singleton instance of the anonymous inode file system.
    fn singleton() -> &'static Arc<NaivePseudoFs> {
        static ANON_INODEFS: Once<Arc<NaivePseudoFs>> = Once::new();

        NaivePseudoFs::singleton(&ANON_INODEFS, "anon_inodefs", ANON_INODEFS_MAGIC)
    }

    /// Creates a pseudo `Path` for the shared inode.
    pub(crate) fn new_path(name_fn: fn(&dyn Inode) -> String) -> Path {
        Path::new_pseudo(
            Self::mount_node().clone(),
            Self::shared_inode().clone(),
            name_fn,
        )
    }

    /// Formats the common fields for a file description backed by the shared inode.
    pub(crate) fn fdinfo(common: &FileCommon, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo(u32);

        impl Display for FdInfo {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                writeln!(f, "pos:\t0")?;
                writeln!(f, "flags:\t0{:o}", self.0)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }

        let mut flags = common.status_flags().bits() | common.access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }
        Box::new(FdInfo(flags))
    }

    /// Returns the pseudo mount node of the anonymous inode file system.
    pub(crate) fn mount_node() -> &'static Arc<Mount> {
        static ANON_INODEFS_MOUNT: Once<Arc<Mount>> = Once::new();

        ANON_INODEFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()).unwrap())
    }

    /// Returns the shared inode of the anonymous inode file system singleton.
    //
    // Some members of anon_inodefs (such as epollfd, eventfd, timerfd, etc.) share
    // the same inode. The sharing is not only within the same category (e.g., two
    // epollfds share the same inode) but also across different categories (e.g.,
    // an epollfd and a timerfd share the same inode). Even across namespaces, this
    // inode is still shared. Although this Linux behavior is a bit odd, we keep it
    // for compatibility.
    //
    // A small subset of members in anon_inodefs (i.e., userfaultfd, io_uring, and
    // kvm_guest_memfd) have their own dedicated inodes. We need to support creating
    // independent inodes within anon_inodefs for them in the future.
    //
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/anon_inodes.c#L153-L164>
    pub(crate) fn shared_inode() -> &'static Arc<dyn Inode> {
        static SHARED_INODE: Once<Arc<dyn Inode>> = Once::new();

        SHARED_INODE.call_once(|| {
            let shared_inode = Self::singleton().alloc_inode(
                PseudoInodeType::AnonInode,
                mkmod!(u+rw),
                Uid::new_root(),
                Gid::new_root(),
            );

            Arc::new(shared_inode)
        })
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/magic.h#L93>
const ANON_INODEFS_MAGIC: u64 = 0x09041934;
