use alloc::string::String;
use core::any::Any;
use core::time::Duration;
use jinux_frame::vm::VmFrame;

use crate::fs::utils::{
    DirEntryVec, DirentVisitor, FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata,
};
use crate::prelude::*;

use super::{ProcFS, ProcInodeInfo};

pub struct ProcDir<D: DirOps> {
    inner: D,
    this: Weak<ProcDir<D>>,
    parent: Option<Weak<dyn Inode>>,
    cached_children: RwLock<DirEntryVec<(String, Arc<dyn Inode>)>>,
    info: ProcInodeInfo,
}

impl<D: DirOps> ProcDir<D> {
    pub fn new(
        dir: D,
        fs: Arc<dyn FileSystem>,
        parent: Option<Weak<dyn Inode>>,
        is_volatile: bool,
    ) -> Arc<Self> {
        let info = {
            let procfs = fs.downcast_ref::<ProcFS>().unwrap();
            let metadata = Metadata::new_dir(
                procfs.alloc_id(),
                InodeMode::from_bits_truncate(0o555),
                &fs.sb(),
            );
            ProcInodeInfo::new(metadata, Arc::downgrade(&fs), is_volatile)
        };
        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent,
            cached_children: RwLock::new(DirEntryVec::new()),
            info,
        })
    }

    pub fn this(&self) -> Arc<ProcDir<D>> {
        self.this.upgrade().unwrap()
    }

    pub fn parent(&self) -> Option<Arc<dyn Inode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }

    pub fn cached_children(&self) -> &RwLock<DirEntryVec<(String, Arc<dyn Inode>)>> {
        &self.cached_children
    }
}

impl<D: DirOps + 'static> Inode for ProcDir<D> {
    fn len(&self) -> usize {
        self.info.metadata().size
    }

    fn resize(&self, _new_size: usize) {}

    fn metadata(&self) -> Metadata {
        self.info.metadata().clone()
    }

    fn atime(&self) -> Duration {
        self.info.metadata().atime
    }

    fn set_atime(&self, _time: Duration) {}

    fn mtime(&self) -> Duration {
        self.info.metadata().mtime
    }

    fn set_mtime(&self, _time: Duration) {}

    fn read_page(&self, _idx: usize, _frame: &VmFrame) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_page(&self, _idx: usize, _frame: &VmFrame) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn mknod(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn readdir_at(&self, mut offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            // Read the two special entries.
            if *offset == 0 {
                let this_inode = self.this();
                visitor.visit(
                    ".",
                    this_inode.info.metadata().ino as u64,
                    this_inode.info.metadata().type_,
                    *offset,
                )?;
                *offset += 1;
            }
            if *offset == 1 {
                let parent_inode = self.parent().unwrap_or(self.this());
                visitor.visit(
                    "..",
                    parent_inode.metadata().ino as u64,
                    parent_inode.metadata().type_,
                    *offset,
                )?;
                *offset += 1;
            }

            // Read the normal child entries.
            self.inner.populate_children(self.this.clone());
            let cached_children = self.cached_children.read();
            for (idx, (name, child)) in cached_children
                .idxes_and_entries()
                .map(|(idx, (name, child))| (idx + 2, (name, child)))
            {
                if idx < *offset {
                    continue;
                }
                visitor.visit(
                    name.as_ref(),
                    child.metadata().ino as u64,
                    child.metadata().type_,
                    idx,
                )?;
                *offset = idx + 1;
            }
            Ok(())
        };

        let initial_offset = offset;
        match try_readdir(&mut offset, visitor) {
            Err(e) if initial_offset == offset => Err(e),
            _ => Ok(offset - initial_offset),
        }
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn unlink(&self, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn rmdir(&self, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "." => self.this(),
            ".." => self.parent().unwrap_or(self.this()),
            name => {
                let mut cached_children = self.cached_children.write();
                if let Some((_, inode)) = cached_children
                    .iter()
                    .find(|(child_name, inode)| child_name.as_str() == name)
                {
                    return Ok(inode.clone());
                }
                let inode = self.inner.lookup_child(self.this.clone(), name)?;
                cached_children.put((String::from(name), inode.clone()));
                inode
            }
        };
        Ok(inode)
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn read_link(&self) -> Result<String> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn ioctl(&self, _cmd: &IoctlCmd) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.info.fs().upgrade().unwrap()
    }

    fn is_dentry_cacheable(&self) -> bool {
        !self.info.is_volatile()
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }
}

pub trait DirOps: Sync + Send {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOENT))
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {}
}
