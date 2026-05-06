// SPDX-License-Identifier: MPL-2.0

//! Reusable procfs directory inode templates and `readdir` helpers.

use alloc::borrow::Cow;
use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use super::Common;
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags},
        procfs::{BLOCK_SIZE, ProcFs},
        utils::DirentVisitor,
        vfs::{
            file_system::{FileSystem, SuperBlock},
            inode::{Extension, Inode, InodeIo, Metadata, MknodType, RevalidationPolicy},
            path::{is_dot, is_dotdot},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

/// Wraps directory-specific procfs operations as a VFS inode.
///
/// `ProcDir` owns the directory implementation object,
/// tracks parent linkage for `.` and `..`,
/// and forwards inode methods that are common to all procfs directories.
pub struct ProcDir<D: DirOps> {
    inner: D,
    this: Weak<ProcDir<D>>,
    parent: Option<Weak<dyn Inode>>,
    common: Common,
}

impl<D: DirOps> ProcDir<D> {
    /// Creates the root procfs directory inode.
    pub fn new_root(
        dir: D,
        fs: Weak<dyn FileSystem>,
        ino: u64,
        sb: &SuperBlock,
        mode: InodeMode,
    ) -> Arc<Self> {
        let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE, sb.container_dev_id);
        let common = Common::new(metadata, fs);

        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent: None,
            common,
        })
    }

    /// Creates a non-root procfs directory inode under `parent`.
    pub fn new(dir: D, parent: Weak<dyn Inode>, mode: InodeMode) -> Arc<Self> {
        let common = {
            let fs = parent.upgrade().unwrap().fs();
            let procfs = fs.downcast_ref::<ProcFs>().unwrap();
            let ino = procfs.alloc_id();
            let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE, procfs.sb().container_dev_id);
            Common::new(metadata, Arc::downgrade(&fs))
        };
        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent: Some(parent),
            common,
        })
    }

    fn this(&self) -> Arc<ProcDir<D>> {
        self.this.upgrade().unwrap()
    }

    pub fn this_weak(&self) -> &Weak<ProcDir<D>> {
        &self.this
    }

    fn parent(&self) -> Option<Arc<dyn Inode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }

    /// Returns the directory-specific procfs operations.
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
        ) -> impl Iterator<Item = (&'static str, Arc<dyn Inode>, usize)> {
            let this_inode: Arc<dyn Inode> = dir.this();
            let parent_inode = dir.parent().unwrap_or_else(|| this_inode.clone());
            [(".", this_inode, 1), ("..", parent_inode, 2)].into_iter()
        }

        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            for (name, inode, next_offset) in special_entries(self) {
                if next_offset <= *offset {
                    continue;
                }

                visitor.visit(name, inode.ino(), inode.type_(), next_offset)?;
                *offset = next_offset;
            }

            self.inner.visit_entries_from_offset(*offset, |child| {
                visitor.visit(
                    child.name.as_ref(),
                    child.ino,
                    child.type_,
                    child.next_offset,
                )?;
                *offset = child.next_offset;
                Ok(())
            })?;
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

    fn revalidation_policy(&self) -> RevalidationPolicy {
        self.inner.revalidation_policy()
    }

    fn revalidate_exists(&self, name: &str, child: &dyn Inode) -> bool {
        self.inner.revalidate_exists(name, child)
    }

    fn revalidate_absent(&self, name: &str) -> bool {
        self.inner.revalidate_absent(name)
    }

    fn seek_end(&self) -> Option<usize> {
        // Seeking directories under `/proc` with `SEEK_END` will start from zero.
        Some(0)
    }
}

pub trait DirOps: Sync + Send + Sized {
    /// Looks up a child inode in `this_dir` by name.
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>>;

    /// Visits readdir entries whose continuation offsets are strictly greater than `offset`.
    ///
    /// Build entries with helpers from the [`readdir`] submodule and forward to
    /// [`readdir::visit_readdir_entries`]. Common patterns:
    ///
    /// - Static or sequential children: [`readdir::sequential_readdir_entries`]
    ///   or the [`readdir::visit_listed_entries`] convenience wrapper.
    /// - Children identified by a stable integer key: [`readdir::keyed_readdir_entries`].
    /// - Mixed children: invoke [`readdir::visit_readdir_entries`] more than
    ///   once per call, once per group. See `RootDirOps`.
    ///
    /// The default implementation reports no entries.
    fn visit_entries_from_offset<'a, F>(&'a self, _offset: usize, _visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        Ok(())
    }

    /// Returns the dentry cache revalidation policy for this directory.
    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::empty()
    }

    /// Checks whether a positive lookup result still exists.
    ///
    /// This method is only consulted when [`Self::revalidation_policy`] includes
    /// `REVALIDATE_EXISTS`.
    #[must_use]
    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        true
    }

    /// Checks whether a negative lookup result is still absent.
    ///
    /// This method is only consulted when [`Self::revalidation_policy`] includes
    /// `REVALIDATE_ABSENT`.
    #[must_use]
    fn revalidate_absent(&self, _name: &str) -> bool {
        true
    }
}

/// A statically declared procfs child entry.
///
/// The tuple stores the exported filename,
/// the inode type reported to [`Inode::readdir_at`],
/// and the constructor used by lookup paths to instantiate the inode.
pub type StaticDirEntry<Fp> = (&'static str, InodeType, Fp);

/// Looks up a statically declared child from a `StaticDirEntry` table.
///
/// The `constructor_adaptor` receives the stored constructor payload
/// and can bind any per-directory context that is needed at the call site.
pub fn lookup_child_from_table<Fp, F>(
    name: &str,
    table: &[StaticDirEntry<Fp>],
    constructor_adaptor: F,
) -> Option<Arc<dyn Inode>>
where
    Fp: Copy,
    F: FnOnce(Fp) -> Arc<dyn Inode>,
{
    table
        .iter()
        .find(|(child_name, _, _)| *child_name == name)
        .map(|(_, _, child_constructor)| (constructor_adaptor)(*child_constructor))
}

/// Converts a static procfs child table into listed entries.
pub fn listed_entries_from_table<'a, Fp>(
    table: &'a [StaticDirEntry<Fp>],
) -> impl Iterator<Item = ListedEntry<'a>> + 'a
where
    Fp: Copy + 'a,
{
    table
        .iter()
        .map(|(name, type_, _)| ListedEntry::new(*name, *type_))
}

mod readdir {
    //! `readdir` resumption types and strategies.
    //!
    //! When user space calls `getdents`, it passes an offset and expects the
    //! kernel to emit entries strictly past it. Each emitted entry carries a
    //! continuation offset that user space remembers for the next call.
    //!
    //! # Vocabulary
    //!
    //! - **Offset** - the cursor passed to `Inode::readdir_at`. Maps to
    //!   `dirent::d_off`. Offsets `1` and `2` are reserved for `.` and `..`.
    //!
    //! - **Key** - an integer that uniquely identifies one dynamic child
    //!   (PID, TID, FD). Comes from kernel state, not iteration order.
    //!
    //! - **Continuation offset** - the offset stored in [`ReaddirEntry`].
    //!   Computed by [`sequential_readdir_entries`] from index or
    //!   [`keyed_readdir_entries`] from key:
    //!
    //!   ```text
    //!   continuation_offset = first_entry_offset + index + 1   // sequential_*
    //!   continuation_offset = first_entry_offset + key   + 1   // keyed_*
    //!   ```

    use super::*;

    /// A listed entry that is currently visible in a procfs directory.
    ///
    /// Unlike [`ReaddirEntry`], this type does not carry a continuation offset.
    /// It is intended for higher-level directory descriptions,
    /// while [`ReaddirEntry`] derives offsets from the iteration strategy.
    pub struct ListedEntry<'a> {
        pub(super) name: Cow<'a, str>,
        pub(super) type_: InodeType,
    }

    impl<'a> ListedEntry<'a> {
        /// Creates a listed entry with the given filename and inode type.
        pub fn new(name: impl Into<Cow<'a, str>>, type_: InodeType) -> Self {
            Self {
                name: name.into(),
                type_,
            }
        }

        /// Returns the filename of the listed entry.
        pub fn name(&self) -> &str {
            self.name.as_ref()
        }
    }

    /// A directory entry emitted by [`DirOps::visit_entries_from_offset`].
    ///
    /// The `next_offset` is the continuation offset that should be used as the
    /// starting point of the next [`Inode::readdir_at`] call.
    pub struct ReaddirEntry<'a> {
        pub(super) name: Cow<'a, str>,
        pub(super) ino: u64,
        pub(super) type_: InodeType,
        pub(super) next_offset: usize,
    }

    impl<'a> ReaddirEntry<'a> {
        /// Creates an entry with an explicit continuation offset.
        pub(super) fn new(
            name: impl Into<Cow<'a, str>>,
            ino: u64,
            type_: InodeType,
            next_offset: usize,
        ) -> Self {
            Self {
                name: name.into(),
                ino,
                type_,
                next_offset,
            }
        }
    }

    /// A placeholder inode number for procfs entries.
    ///
    /// [`Inode::readdir_at`] reports procfs entries without instantiating child inodes first,
    /// so the inode number returned here is only a placeholder.
    /// Lookup still creates the real child inode on demand when the path is opened.
    //
    // TODO: Adopt deterministic inode numbers derived from stable entity identity
    // (for example, PID) as the long-term fix for `d_ino`/`st_ino` consistency.
    pub(super) const PLACEHOLDER_DIRENT_INO: u64 = 2;

    /// Builds sequential directory entries whose continuation offsets increase by one.
    ///
    /// This helper is appropriate for directories whose visible children already
    /// have a deterministic iteration order.
    /// It reserves offsets `1` and `2` for `.` and `..`,
    /// so callers typically pass `2` as `first_entry_offset`.
    pub fn sequential_readdir_entries<'a, I>(
        offset: usize,
        first_entry_offset: usize,
        entries: I,
    ) -> impl Iterator<Item = ReaddirEntry<'a>>
    where
        I: IntoIterator<Item = ListedEntry<'a>>,
    {
        entries
            .into_iter()
            .enumerate()
            .filter_map(move |(idx, entry)| {
                let next_offset = first_entry_offset.saturating_add(idx).saturating_add(1);
                let ListedEntry { name, type_ } = entry;
                (next_offset > offset)
                    .then(|| ReaddirEntry::new(name, PLACEHOLDER_DIRENT_INO, type_, next_offset))
            })
    }

    /// Builds directory entries whose offsets are derived from stable integer keys.
    ///
    /// This is useful for procfs directories such as `/proc`, `/proc/[pid]/task`,
    /// and `/proc/[pid]/fd`, where Linux uses identifiers like PID, TID, or FD to
    /// keep iteration stable across mutations.
    ///
    /// Names are produced only after the corresponding key passes the offset check.
    /// The resulting continuation offset for key `k` is `first_entry_offset + k + 1`,
    /// so callers should provide non-negative keys in increasing order.
    /// Keys do not need to be dense,
    /// but each key must stay stable for the duration of one directory snapshot.
    pub fn keyed_readdir_entries<'a, I, F>(
        offset: usize,
        first_entry_offset: usize,
        keys: I,
        mut entry_fn: F,
    ) -> impl Iterator<Item = ReaddirEntry<'a>>
    where
        I: IntoIterator<Item = usize>,
        F: FnMut(usize) -> ListedEntry<'a>,
    {
        let start_key = offset.saturating_sub(first_entry_offset);
        keys.into_iter().filter_map(move |key| {
            let next_offset = first_entry_offset.saturating_add(key).saturating_add(1);
            (key >= start_key && next_offset > offset).then(|| {
                let ListedEntry { name, type_ } = entry_fn(key);
                ReaddirEntry::new(name, PLACEHOLDER_DIRENT_INO, type_, next_offset)
            })
        })
    }

    /// Visits the given [`ReaddirEntry`] values in order, calling `visit_fn` for each one.
    pub fn visit_readdir_entries<'a, I, F>(entries: I, mut visit_fn: F) -> Result<()>
    where
        I: IntoIterator<Item = ReaddirEntry<'a>>,
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        for entry in entries {
            visit_fn(entry)?;
        }

        Ok(())
    }

    /// Visits listed entries using sequential continuation offsets.
    pub fn visit_listed_entries<'a, I, F>(offset: usize, listed: I, visit_fn: F) -> Result<()>
    where
        I: IntoIterator<Item = ListedEntry<'a>>,
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_readdir_entries(sequential_readdir_entries(offset, 2, listed), visit_fn)
    }
}

pub use readdir::{
    ListedEntry, ReaddirEntry, keyed_readdir_entries, sequential_readdir_entries,
    visit_listed_entries, visit_readdir_entries,
};
