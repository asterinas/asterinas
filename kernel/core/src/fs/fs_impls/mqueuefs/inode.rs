// SPDX-License-Identifier: MPL-2.0

//! Inode implementations of `mqueuefs`.

use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use super::{BLOCK_SIZE, ROOT_INO};
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags, mkmod},
        utils::DirentVisitor,
        vfs::{
            file_system::{FileSystem, SuperBlock},
            inode::{Extension, FileOps, Inode, Metadata, MknodType, RenameMode},
            path::{Dentry, is_dot, is_dotdot},
        },
    },
    ipc::mqueue::posix::MqueueQueue,
    prelude::*,
    process::{Gid, Uid},
};

/// Shared inode state common to all `mqueuefs` inodes.
struct Common {
    metadata: RwLock<Metadata>,
    extension: Extension,
    fs: Weak<dyn FileSystem>,
}

impl Common {
    fn new(metadata: Metadata, fs: Weak<dyn FileSystem>) -> Self {
        Self {
            metadata: RwLock::new(metadata),
            extension: Extension::new(),
            fs,
        }
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    fn ino(&self) -> u64 {
        self.metadata.read().ino
    }

    fn size(&self) -> usize {
        self.metadata.read().size
    }

    fn metadata(&self) -> Result<Metadata> {
        Ok(*self.metadata.read())
    }

    fn atime(&self) -> Duration {
        self.metadata.read().last_access_at
    }

    fn set_atime(&self, _self_dentry: &Dentry, time: Duration) {
        self.metadata.write().last_access_at = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata.read().last_modify_at
    }

    fn set_mtime(&self, _self_dentry: &Dentry, time: Duration) {
        self.metadata.write().last_modify_at = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata.read().last_meta_change_at
    }

    fn set_ctime(&self, _self_dentry: &Dentry, time: Duration) {
        self.metadata.write().last_meta_change_at = time;
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.read().mode)
    }

    fn set_mode(&self, _self_dentry: &Dentry, mode: InodeMode) -> Result<()> {
        self.metadata.write().mode = mode;
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.read().uid)
    }

    fn set_owner(&self, _self_dentry: &Dentry, uid: Uid) -> Result<()> {
        self.metadata.write().uid = uid;
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.read().gid)
    }

    fn set_group(&self, _self_dentry: &Dentry, gid: Gid) -> Result<()> {
        self.metadata.write().gid = gid;
        Ok(())
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }
}

/// The root directory inode of `mqueuefs`.
pub(super) struct MqueueRootInode {
    common: Common,
    this: Weak<MqueueRootInode>,
    parent: Option<Weak<dyn Inode>>,
}

impl MqueueRootInode {
    pub(super) fn new_root(fs: Weak<dyn FileSystem>, sb: &SuperBlock) -> Arc<Self> {
        let metadata =
            Metadata::new_dir(ROOT_INO, mkmod!(a+rx, u+w), BLOCK_SIZE, sb.container_dev_id);
        let common = Common::new(metadata, fs);

        Arc::new_cyclic(|weak_self| Self {
            common,
            this: weak_self.clone(),
            parent: None,
        })
    }

    fn this(&self) -> Arc<dyn Inode> {
        self.this.upgrade().unwrap()
    }

    fn parent(&self) -> Option<Arc<dyn Inode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }
}

impl FileOps for MqueueRootInode {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let this_inode: Arc<dyn Inode> = self.this();
        let parent_inode = self.parent().unwrap_or_else(|| this_inode.clone());
        let entries = [(".", this_inode, 1usize), ("..", parent_inode, 2usize)];

        let mut iterate_offset = offset;
        for (name, inode, next_offset) in entries {
            if next_offset <= iterate_offset {
                continue;
            }

            visitor.visit(name, inode.ino(), inode.type_(), next_offset)?;
            iterate_offset = next_offset;
        }

        Ok(iterate_offset - offset)
    }
}

#[inherit_methods(from = "self.common")]
impl Inode for MqueueRootInode {
    fn size(&self) -> usize;
    fn extension(&self) -> &Extension;
    fn ino(&self) -> u64;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, self_dentry: &Dentry, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, self_dentry: &Dentry, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, self_dentry: &Dentry, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, self_dentry: &Dentry, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, self_dentry: &Dentry, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;
    fn metadata(&self) -> Result<Metadata>;

    fn resize(&self, _self_dentry: &Dentry, _new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn type_(&self) -> InodeType {
        InodeType::Dir
    }

    fn create(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _type_: InodeType,
        _mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn mknod(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _mode: InodeMode,
        _type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn link(&self, _self_dentry: &Dentry, _old_dentry: &Dentry, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn unlink(&self, _child_dentry: &Dentry) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn rmdir(&self, _child_dentry: &Dentry) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if is_dot(name) {
            return Ok(self.this());
        }
        if is_dotdot(name) {
            return Ok(self.parent().unwrap_or_else(|| self.this()));
        }

        // TODO: Look up queue files once `mq_open` is implemented.
        return_errno!(Errno::ENOENT)
    }

    fn rename(
        &self,
        _old_child_dentry: &Dentry,
        _new_dir_dentry: &Dentry,
        _new_name: &str,
        _replaced_inode: Option<&Arc<dyn Inode>>,
        _mode: RenameMode,
    ) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }
}

/// A queue file inode, wrapping the core queue state.
pub(crate) struct MqueueInode {
    queue: Arc<MqueueQueue>,
    common: Common,
}

impl MqueueInode {
    pub(crate) fn new(
        ino: u64,
        mode: InodeMode,
        queue: Arc<MqueueQueue>,
        fs: Weak<dyn FileSystem>,
        sb: &SuperBlock,
    ) -> Arc<Self> {
        let mut metadata = Metadata::new_file(ino, mode, BLOCK_SIZE, sb.container_dev_id);
        // The queue file is owned by the user who created the queue, so that the
        // permission checks of `mq_open` and `mq_unlink` agree on the owner.
        metadata.uid = queue.owner();
        metadata.gid = queue.group();
        let common = Common::new(metadata, fs);
        Arc::new(Self { queue, common })
    }

    pub(crate) fn queue(&self) -> &Arc<MqueueQueue> {
        &self.queue
    }
}

impl FileOps for MqueueInode {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        // Message queues are accessed via `mq_timedsend` / `mq_timedreceive`,
        // not via `read` / `write`.
        return_errno!(Errno::EINVAL)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno!(Errno::EINVAL)
    }
}

#[inherit_methods(from = "self.common")]
impl Inode for MqueueInode {
    fn extension(&self) -> &Extension;
    fn ino(&self) -> u64;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, self_dentry: &Dentry, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, self_dentry: &Dentry, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, self_dentry: &Dentry, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, self_dentry: &Dentry, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, self_dentry: &Dentry, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;

    /// The size of a queue file is the total payload bytes currently queued.
    fn size(&self) -> usize {
        self.queue.qsize()
    }

    /// Returns metadata with the queue's current data size.
    fn metadata(&self) -> Result<Metadata> {
        let mut metadata = self.common.metadata()?;
        metadata.size = self.queue.qsize();
        Ok(metadata)
    }

    fn resize(&self, _self_dentry: &Dentry, _new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EINVAL))
    }

    fn type_(&self) -> InodeType {
        InodeType::File
    }
}
