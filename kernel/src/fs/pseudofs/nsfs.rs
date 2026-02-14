// SPDX-License-Identifier: MPL-2.0

use alloc::format;
use core::time::Duration;

use inherit_methods_macro::inherit_methods;
use ostd::task::Task;
use spin::Once;

use crate::{
    events::IoEvents,
    fs::{
        file_table::{FdFlags, FileDesc},
        inode_handle::{FileIo, InodeHandle},
        path::{Mount, Path},
        pseudofs::{PseudoFs, PseudoInode, PseudoInodeType},
        utils::{
            AccessMode, Extension, FileSystem, Inode, InodeIo, InodeMode, InodeType, Metadata,
            StatusFlags, mkmod,
        },
    },
    prelude::*,
    process::{
        CloneFlags, Gid, Uid, UserNamespace,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

/// A pseudo filesystem for namespace files.
pub struct NsFs {
    _private: (),
}

impl NsFs {
    /// Returns the singleton instance of the ns filesystem.
    pub fn singleton() -> &'static Arc<PseudoFs> {
        static NSFS: Once<Arc<PseudoFs>> = Once::new();
        PseudoFs::singleton(&NSFS, "nsfs", NSFS_MAGIC)
    }

    /// Creates a pseudo [`Path`] for a namespace file.
    pub fn new_path<T: NsCommonOps>(ns: Weak<T>) -> Path {
        let ns_inode = {
            let ino = Self::singleton().alloc_id();
            let fs = Arc::downgrade(Self::singleton());
            Arc::new(NsInode::new(ino, Uid::new_root(), Gid::new_root(), ns, fs))
        };

        Path::new_pseudo(Self::mount_node().clone(), ns_inode, |inode| {
            let inode = inode.downcast_ref::<NsInode<T>>().unwrap();
            inode.name().to_string()
        })
    }

    /// Returns the pseudo mount node of the ns filesystem.
    pub fn mount_node() -> &'static Arc<Mount> {
        static NSFS_MOUNT: Once<Arc<Mount>> = Once::new();
        NSFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()))
    }
}

/// An inode representing a namespace entry in [`NsFs`].
struct NsInode<T: NsCommonOps> {
    common: PseudoInode,
    ns: Weak<T>,
    name: String,
}

impl<T: NsCommonOps> NsInode<T> {
    fn new(ino: u64, uid: Uid, gid: Gid, ns: Weak<T>, fs: Weak<PseudoFs>) -> Self {
        let mode = mkmod!(a+r);
        let common = PseudoInode::new(ino, PseudoInodeType::Ns, mode, uid, gid, fs);
        let name = format!("{}:[{}]", T::NAME, ino);

        Self { common, ns, name }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[inherit_methods(from = "self.common")]
impl<T: NsCommonOps> Inode for NsInode<T> {
    fn size(&self) -> usize;
    fn resize(&self, _new_size: usize) -> Result<()>;
    fn metadata(&self) -> Metadata;
    fn extension(&self) -> &Extension;
    fn ino(&self) -> u64;
    fn type_(&self) -> InodeType;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;

    fn open(
        &self,
        access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        // FIXME: This may not be the most appropriate place to check the access mode,
        // but the check must not be bypassed even if the current process has the
        // CAP_DAC_OVERRIDE capability. It is hard to find a better place for it,
        // and an extra check here does no harm.
        if access_mode.is_writable() {
            return Some(Err(Error::with_message(
                Errno::EPERM,
                "ns files cannot be opened as writable",
            )));
        }

        let ns = self
            .ns
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "the namespace no longer exists"));

        Some(ns.map(|ns| Box::new(NsFile { ns }) as Box<dyn FileIo>))
    }
}

#[inherit_methods(from = "self.common")]
impl<T: NsCommonOps> InodeIo for NsInode<T> {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status: StatusFlags,
    ) -> Result<usize>;
    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status: StatusFlags,
    ) -> Result<usize>;
}

/// A file handle referencing a live namespace.
pub struct NsFile<T: NsCommonOps> {
    ns: Arc<T>,
}

impl<T: NsCommonOps> NsFile<T> {
    /// Returns a reference to the underlying namespace.
    pub fn ns(&self) -> &Arc<T> {
        &self.ns
    }
}

impl<T: NsCommonOps> FileIo for NsFile<T> {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "ns files are not seekable");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;
        dispatch_ioctl!(match raw_ioctl {
            _cmd @ GetUserNs => {
                let user_ns = self.ns.get_owner_user_ns()?;

                let current = current!();
                let current_user_ns = current.user_ns().lock();
                if !current_user_ns.is_same_or_ancestor_of(user_ns) {
                    return_errno_with_message!(
                        Errno::EPERM,
                        "the owner user namespace is not an ancestor of the current namespace"
                    );
                }

                open_ns_as_file(user_ns.as_ref())
            }
            _cmd @ GetParent => {
                let parent = self.ns.get_parent()?;
                open_ns_as_file(parent.as_ref())
            }
            _cmd @ GetType => {
                let clone_flags = CloneFlags::from(T::TYPE);
                Ok(clone_flags.bits().cast_signed())
            }
            cmd @ GetOwnerUid => {
                let ns = self.ns.as_ref() as &dyn Any;
                let user_ns = ns.downcast_ref::<UserNamespace>().ok_or_else(|| {
                    Error::with_message(
                        Errno::EINVAL,
                        "the ns file does not correspond to a user namespace",
                    )
                })?;
                let uid = user_ns.get_owner_uid()?;
                cmd.write(&uid.into())?;
                Ok(0)
            }
            // TODO: Support additional ioctl commands
            _ => return_errno_with_message!(Errno::ENOTTY, "unsupported ioctl command"),
        })
    }
}

impl<T: NsCommonOps> Pollable for NsFile<T> {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        (IoEvents::IN | IoEvents::OUT | IoEvents::RDNORM) & mask
    }
}

impl<T: NsCommonOps> InodeIo for NsFile<T> {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "ns files do not support read_at");
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "ns files do not support write_at");
    }
}

/// Opens a namespace as a file and returns the file descriptor.
fn open_ns_as_file<T: NsCommonOps>(ns: &T) -> Result<FileDesc> {
    let path = ns.path();
    let inode_handle = InodeHandle::new(path.clone(), AccessMode::O_RDONLY, StatusFlags::empty())?;

    let current_task = Task::current().unwrap();
    let thread_local = current_task.as_thread_local().unwrap();
    let mut file_table_ref = thread_local.borrow_file_table_mut();
    let mut file_table = file_table_ref.unwrap().write();
    let fd = file_table.insert(Arc::new(inode_handle), FdFlags::CLOEXEC);

    Ok(fd)
}

/// Common operations shared by all namespace types.
///
/// Implementors represent a specific kind of namespace (e.g., UTS, mount, user)
/// and must provide the associated metadata and traversal methods required by
/// [`NsFs`] and [`NsFile`].
pub trait NsCommonOps: Any + Send + Sync + 'static {
    /// The human-readable name of this namespace kind (derived from [`Self::TYPE`]).
    const NAME: &str = Self::TYPE.as_str();

    /// The [`NsType`] discriminant for this namespace kind.
    const TYPE: NsType;

    /// Returns the owner user namespace.
    fn get_owner_user_ns(&self) -> Result<&Arc<UserNamespace>>;

    /// Returns the parent namespace, if one exists.
    fn get_parent(&self) -> Result<Arc<Self>>;

    /// Returns the pseudo filesystem [`Path`] associated with this namespace.
    fn path(&self) -> &Path;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NsType {
    Uts,
    User,
    Mnt,
    #[expect(unused)]
    Pid,
    #[expect(unused)]
    Time,
    #[expect(unused)]
    Cgroup,
    #[expect(unused)]
    Ipc,
    #[expect(unused)]
    Net,
}

impl NsType {
    const fn as_str(&self) -> &'static str {
        match self {
            NsType::Uts => "uts",
            NsType::User => "user",
            NsType::Mnt => "mnt",
            NsType::Pid => "pid",
            NsType::Time => "time",
            NsType::Cgroup => "cgroup",
            NsType::Ipc => "ipc",
            NsType::Net => "net",
        }
    }
}

impl From<NsType> for CloneFlags {
    fn from(value: NsType) -> Self {
        match value {
            NsType::Uts => CloneFlags::CLONE_NEWUTS,
            NsType::User => CloneFlags::CLONE_NEWUSER,
            NsType::Mnt => CloneFlags::CLONE_NEWNS,
            NsType::Pid => CloneFlags::CLONE_NEWPID,
            NsType::Time => CloneFlags::CLONE_NEWTIME,
            NsType::Cgroup => CloneFlags::CLONE_NEWCGROUP,
            NsType::Ipc => CloneFlags::CLONE_NEWIPC,
            NsType::Net => CloneFlags::CLONE_NEWNET,
        }
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/magic.h#L95>
const NSFS_MAGIC: u64 = 0x6e736673;

mod ioctl_defs {
    use crate::util::ioctl::{InData, NoData, OutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/nsfs.h#L10>

    // Legacy encoding ioctl commands

    /// Returns a file descriptor of the owner user namespace.
    pub type GetUserNs       = ioc!(NS_GET_USERNS,    0xb701, NoData);
    /// Returns a file descriptor of the parent namespace.
    pub type GetParent       = ioc!(NS_GET_PARENT,     0xb702, NoData);
    /// Gets the type of the namespace (e.g., user, pid, mnt, etc.).
    pub type GetType         = ioc!(NS_GET_NSTYPE,     0xb703, NoData);
    /// Gets the user ID of the namespace owner.
    ///
    /// Only user namespace supports this operation.
    pub type GetOwnerUid     = ioc!(NS_GET_OWNER_UID,  0xb704, OutData<u32>);

    // Modern encoding ioctl commands

    /// Gets the ID of the mount namespace.
    #[expect(unused)]
    pub type GetMntNsId      = ioc!(NS_GET_MNTNS_ID,        0xb7, 0x5, OutData<u64>);
    /// Translates thread ID from the target PID namespace into the caller's PID namespace.
    #[expect(unused)]
    pub type GetTidFromPidNs = ioc!(NS_GET_PID_FROM_PIDNS,  0xb7, 0x6, InData<i32>);
    /// Translates process ID from the target PID namespace into the caller's PID namespace.
    #[expect(unused)]
    pub type GetPidFromPidNs = ioc!(NS_GET_TGID_FROM_PIDNS, 0xb7, 0x7, InData<i32>);
    /// Translates thread ID from the caller's PID namespace into the target PID namespace.
    #[expect(unused)]
    pub type GetTidInPidNs   = ioc!(NS_GET_PID_IN_PIDNS,    0xb7, 0x8, InData<i32>);
    /// Translates process ID from the caller's PID namespace into the target PID namespace.
    #[expect(unused)]
    pub type GetPidInPidNs   = ioc!(NS_GET_TGID_IN_PIDNS,   0xb7, 0x9, InData<i32>);
}
