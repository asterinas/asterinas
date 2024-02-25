// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_util::slot_vec::SlotVec;
use inherit_methods_macro::inherit_methods;

use super::{Common, ProcFS};
use crate::{
    fs::{
        device::Device,
        utils::{DirentVisitor, FileSystem, Inode, InodeMode, InodeType, Metadata},
    },
    prelude::*,
    process::{Gid, Uid},
};

pub struct ProcDir<D: DirOps> {
    inner: D,
    this: Weak<ProcDir<D>>,
    parent: Option<Weak<dyn Inode>>,
    cached_children: RwLock<SlotVec<(String, Arc<dyn Inode>)>>,
    common: Common,
}

impl<D: DirOps> ProcDir<D> {
    pub fn new(
        dir: D,
        fs: Arc<dyn FileSystem>,
        parent: Option<Weak<dyn Inode>>,
        is_volatile: bool,
    ) -> Arc<Self> {
        let common = {
            let procfs = fs.downcast_ref::<ProcFS>().unwrap();
            let metadata = Metadata::new_dir(
                procfs.alloc_id(),
                InodeMode::from_bits_truncate(0o555),
                &fs.sb(),
            );
            Common::new(metadata, Arc::downgrade(&fs), is_volatile)
        };
        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent,
            cached_children: RwLock::new(SlotVec::new()),
            common,
        })
    }

    pub fn this(&self) -> Arc<ProcDir<D>> {
        self.this.upgrade().unwrap()
    }

    pub fn parent(&self) -> Option<Arc<dyn Inode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }

    pub fn cached_children(&self) -> &RwLock<SlotVec<(String, Arc<dyn Inode>)>> {
        &self.cached_children
    }
}

#[inherit_methods(from = "self.common")]
impl<D: DirOps + 'static> Inode for ProcDir<D> {
    fn size(&self) -> usize;
    fn metadata(&self) -> Metadata;
    fn ino(&self) -> u64;
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
    fn fs(&self) -> Arc<dyn FileSystem>;

    fn resize(&self, _new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn type_(&self) -> InodeType {
        InodeType::Dir
    }

    fn create(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn mknod(
        &self,
        _name: &str,
        _mode: InodeMode,
        _device: Arc<dyn Device>,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            // Read the two special entries.
            if *offset == 0 {
                let this_inode = self.this();
                visitor.visit(
                    ".",
                    this_inode.common.metadata().ino as u64,
                    this_inode.common.metadata().type_,
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
            let start_offset = *offset;
            for (idx, (name, child)) in cached_children
                .idxes_and_items()
                .map(|(idx, (name, child))| (idx + 2, (name, child)))
                .skip_while(|(idx, _)| idx < &start_offset)
            {
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

        let mut iterate_offset = offset;
        match try_readdir(&mut iterate_offset, visitor) {
            Err(e) if iterate_offset == offset => Err(e),
            _ => Ok(iterate_offset - offset),
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

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn is_dentry_cacheable(&self) -> bool {
        !self.common.is_volatile()
    }
}

pub trait DirOps: Sync + Send {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOENT))
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {}
}
