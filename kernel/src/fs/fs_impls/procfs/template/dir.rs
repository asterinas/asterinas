// SPDX-License-Identifier: MPL-2.0

use core::{iter, time::Duration};

use inherit_methods_macro::inherit_methods;

use super::Common;
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags},
        procfs::{BLOCK_SIZE, ProcFs},
        utils::DirentVisitor,
        vfs::{
            file_system::{FileSystem, SuperBlock},
            inode::{Extension, Inode, InodeIo, Metadata, MknodType},
            path::{is_dot, is_dotdot},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

/// Represents one directory entry emitted by `readdir`.
///
/// The `offset` is the position cookie used as the starting point of
/// the next `readdir` call.
pub struct ReaddirEntry {
    name: String,
    inode: Arc<dyn Inode>,
    offset: usize,
}

impl ReaddirEntry {
    pub fn new(name: String, inode: Arc<dyn Inode>, offset: usize) -> Self {
        Self {
            name,
            inode,
            offset,
        }
    }
}

/// Builds sequential directory entries whose offsets increase by one.
pub fn sequential_readdir_entries<I>(
    offset: usize,
    first_entry_offset: usize,
    entries: I,
) -> Vec<ReaddirEntry>
where
    I: IntoIterator<Item = (String, Arc<dyn Inode>)>,
{
    entries
        .into_iter()
        .enumerate()
        .filter_map(|(idx, (name, inode))| {
            let entry_offset = first_entry_offset.saturating_add(idx);
            (entry_offset >= offset).then(|| ReaddirEntry::new(name, inode, entry_offset))
        })
        .collect()
}

/// Builds directory entries whose offsets are derived from stable integer keys.
///
/// This is useful for procfs directories such as `/proc`, `/proc/[pid]/task`,
/// and `/proc/[pid]/fd`, where Linux uses identifiers like PID, TID, or FD to
/// keep iteration stable across mutations.
pub fn keyed_readdir_entries<I>(
    offset: usize,
    first_entry_offset: usize,
    entries: I,
) -> Vec<ReaddirEntry>
where
    I: IntoIterator<Item = (usize, String, Arc<dyn Inode>)>,
{
    let start_key = offset.saturating_sub(first_entry_offset);
    entries
        .into_iter()
        .filter_map(|(key, name, inode)| {
            (key >= start_key)
                .then(|| ReaddirEntry::new(name, inode, first_entry_offset.saturating_add(key)))
        })
        .collect()
}

pub struct ProcDir<D: DirOps> {
    inner: D,
    this: Weak<ProcDir<D>>,
    parent: Option<Weak<dyn Inode>>,
    common: Common,
    need_neg_child_revalidation: bool,
}

impl<D: DirOps> ProcDir<D> {
    pub(in crate::fs::fs_impls::procfs) fn new_root(
        dir: D,
        fs: Weak<dyn FileSystem>,
        ino: u64,
        sb: &SuperBlock,
        mode: InodeMode,
        need_neg_child_revalidation: bool,
    ) -> Arc<Self> {
        let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE, sb.container_dev_id);
        let common = Common::new(metadata, fs, false);

        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent: None,
            common,
            need_neg_child_revalidation,
        })
    }

    pub(super) fn new(
        dir: D,
        fs: Weak<dyn FileSystem>,
        parent: Option<Weak<dyn Inode>>,
        ino: Option<u64>,
        need_revalidation: bool,
        need_neg_child_revalidation: bool,
        mode: InodeMode,
    ) -> Arc<Self> {
        let common = {
            let arc_fs = fs.upgrade().unwrap();
            let procfs = arc_fs.downcast_ref::<ProcFs>().unwrap();
            let ino = ino.unwrap_or_else(|| procfs.alloc_id());
            let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE, procfs.sb().container_dev_id);
            Common::new(metadata, fs, need_revalidation)
        };
        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent,
            common,
            need_neg_child_revalidation,
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

    pub fn inner(&self) -> &D {
        &self.inner
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
        /// Returns the always-present `.` and `..` entries for a procfs directory.
        fn special_entries<D: DirOps + 'static>(
            dir: &ProcDir<D>,
        ) -> impl Iterator<Item = ReaddirEntry> {
            let this_inode = dir.this();
            let parent_inode = dir.parent().unwrap_or(dir.this());
            iter::once(ReaddirEntry::new(".".to_string(), this_inode, 0)).chain(iter::once(
                ReaddirEntry::new("..".to_string(), parent_inode, 1),
            ))
        }

        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            for child in special_entries(self).chain(self.inner.entries_from_offset(self, *offset))
            {
                if child.offset < *offset {
                    continue;
                }

                visitor.visit(
                    child.name.as_ref(),
                    child.inode.ino(),
                    child.inode.type_(),
                    child.offset,
                )?;
                *offset = child.offset.saturating_add(1);
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

        self.inner.lookup_child(self, name)
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn need_revalidation(&self) -> bool {
        self.common.need_revalidation()
    }

    fn need_neg_child_revalidation(&self) -> bool {
        self.need_neg_child_revalidation
    }

    fn revalidate_pos_child(&self, name: &str, child: &Arc<dyn Inode>) -> bool {
        self.inner.revalidate_pos_child(name, child.as_ref())
    }

    fn revalidate_neg_child(&self, name: &str) -> bool {
        self.inner.revalidate_neg_child(name)
    }

    fn seek_end(&self) -> Option<usize> {
        Some(0)
    }
}

pub trait DirOps: Sync + Send + Sized {
    /// Look up a child inode in given `dir` by name.
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>>;

    /// Returns a snapshot of child entries.
    fn populate_children(&self, dir: &ProcDir<Self>) -> Vec<(String, Arc<dyn Inode>)>;

    /// Returns readdir entries whose offsets are greater than or equal to `offset`.
    ///
    /// Implementations should use stable cookies for dynamic directories so
    /// that iteration can continue correctly after concurrent mutations.
    fn entries_from_offset(&self, dir: &ProcDir<Self>, offset: usize) -> Vec<ReaddirEntry> {
        sequential_readdir_entries(offset, 2, self.populate_children(dir))
    }

    /// Revalidates a positive lookup result.
    #[must_use]
    fn revalidate_pos_child(&self, _name: &str, _child: &dyn Inode) -> bool {
        true
    }

    /// Revalidates a negative lookup result.
    #[must_use]
    fn revalidate_neg_child(&self, _name: &str) -> bool {
        false
    }
}

pub fn lookup_child_from_table<Fp, F>(
    name: &str,
    table: &[(&str, Fp)],
    constructor_adaptor: F,
) -> Option<Arc<dyn Inode>>
where
    Fp: Copy,
    F: FnOnce(Fp) -> Arc<dyn Inode>,
{
    for (child_name, child_constructor) in table.iter() {
        if *child_name == name {
            return Some((constructor_adaptor)(*child_constructor));
        }
    }

    None
}

pub fn populate_children_from_table<Fp, F>(
    children: &mut Vec<(String, Arc<dyn Inode>)>,
    table: &[(&str, Fp)],
    constructor_adaptor: F,
) where
    Fp: Copy,
    F: Fn(Fp) -> Arc<dyn Inode>,
{
    for (child_name, child_constructor) in table.iter() {
        children.push((
            String::from(*child_name),
            (constructor_adaptor)(*child_constructor),
        ));
    }
}
