// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_util::slot_vec::SlotVec;
use inherit_methods_macro::inherit_methods;
use ostd::sync::RwMutexUpgradeableGuard;

use super::{Common, ProcFs};
use crate::{
    fs::{
        path::{is_dot, is_dotdot},
        utils::{
            DirEntryVecExt, DirentVisitor, Extension, FileSystem, Inode, InodeIo, InodeMode,
            InodeType, Metadata, MknodType, StatusFlags,
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

pub struct ProcDir<D: DirOps> {
    inner: D,
    this: Weak<ProcDir<D>>,
    parent: Option<Weak<dyn Inode>>,
    cached_children: RwMutex<SlotVec<(String, Arc<dyn Inode>)>>,
    common: Common,
}

impl<D: DirOps> ProcDir<D> {
    pub(super) fn new(
        dir: D,
        fs: Weak<dyn FileSystem>,
        parent: Option<Weak<dyn Inode>>,
        ino: Option<u64>,
        is_volatile: bool,
        mode: InodeMode,
    ) -> Arc<Self> {
        let common = {
            let ino = ino.unwrap_or_else(|| {
                let arc_fs = fs.upgrade().unwrap();
                let procfs = arc_fs.downcast_ref::<ProcFs>().unwrap();
                procfs.alloc_id()
            });
            let metadata = Metadata::new_dir(ino, mode, super::BLOCK_SIZE);
            Common::new(metadata, fs, is_volatile)
        };
        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent,
            cached_children: RwMutex::new(SlotVec::new()),
            common,
        })
    }

    pub fn this(&self) -> Arc<ProcDir<D>> {
        self.this.upgrade().unwrap()
    }

    pub fn this_weak(&self) -> &Weak<ProcDir<D>> {
        &self.this
    }

    pub fn parent(&self) -> Option<Arc<dyn Inode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }

    pub fn cached_children(&self) -> &RwMutex<SlotVec<(String, Arc<dyn Inode>)>> {
        &self.cached_children
    }
}

impl<D: DirOps + 'static> InodeIo for ProcDir<D> {
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
}

#[inherit_methods(from = "self.common")]
impl<D: DirOps + 'static> Inode for ProcDir<D> {
    fn size(&self) -> usize;
    fn metadata(&self) -> Metadata;
    fn extension(&self) -> &Extension;
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
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, time: Duration);
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

    fn mknod(&self, _name: &str, _mode: InodeMode, _type_: MknodType) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            // Read the two special entries.
            if *offset == 0 {
                let this_inode = self.this();
                visitor.visit(
                    ".",
                    this_inode.common.ino(),
                    this_inode.common.type_(),
                    *offset,
                )?;
                *offset += 1;
            }
            if *offset == 1 {
                let parent_inode = self.parent().unwrap_or(self.this());
                visitor.visit("..", parent_inode.ino(), parent_inode.type_(), *offset)?;
                *offset += 1;
            }

            // Read the normal child entries.
            let cached_children = self.inner.populate_children(self);
            let start_offset = *offset;
            for (idx, (name, child)) in cached_children
                .idxes_and_items()
                .map(|(idx, (name, child))| (idx + 2, (name, child)))
                .skip_while(|(idx, _)| idx < &start_offset)
            {
                visitor.visit(name.as_ref(), child.ino(), child.type_(), idx)?;
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
        if is_dot(name) {
            return Ok(self.this());
        }
        if is_dotdot(name) {
            return Ok(self.parent().unwrap_or(self.this()));
        }

        let cached_children = self.cached_children.read();
        if let Some(inode) = cached_children.find_entry_by_name(name)
            && self.inner.validate_child(inode.as_ref())
        {
            return Ok(inode.clone());
        }
        drop(cached_children);

        self.inner.lookup_child(self, name)
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn is_dentry_cacheable(&self) -> bool {
        !self.common.is_volatile()
    }

    fn seek_end(&self) -> Option<usize> {
        // Seeking directories under `/proc` with `SEEK_END` will start from zero.
        Some(0)
    }
}

pub trait DirOps: Sync + Send + Sized {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>>;

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>>;

    #[must_use]
    fn validate_child(&self, _child: &dyn Inode) -> bool {
        true
    }
}

pub fn lookup_child_from_table<Fp, F>(
    name: &str,
    cached_children: &mut SlotVec<(String, Arc<dyn Inode>)>,
    table: &[(&str, Fp)],
    constructor_adaptor: F,
) -> Option<Arc<dyn Inode>>
where
    Fp: Copy,
    F: FnOnce(Fp) -> Arc<dyn Inode>,
{
    for (child_name, child_constructor) in table.iter() {
        if *child_name == name {
            return Some(
                cached_children
                    .put_entry_if_not_found(name, || (constructor_adaptor)(*child_constructor))
                    .clone(),
            );
        }
    }

    None
}

pub fn populate_children_from_table<Fp, F>(
    cached_children: &mut SlotVec<(String, Arc<dyn Inode>)>,
    table: &[(&str, Fp)],
    constructor_adaptor: F,
) where
    Fp: Copy,
    F: Fn(Fp) -> Arc<dyn Inode>,
{
    for (child_name, child_constructor) in table.iter() {
        cached_children
            .put_entry_if_not_found(child_name, || (constructor_adaptor)(*child_constructor));
    }
}
