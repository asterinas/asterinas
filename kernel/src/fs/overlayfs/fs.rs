// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use alloc::format;
use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use align_ext::AlignExt;
use aster_block::BLOCK_SIZE;
use aster_rights::Full;
use hashbrown::HashSet;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{FrameAllocOptions, UntypedMem};

use crate::{
    fs::{
        device::Device,
        fs_resolver::{FsPath, AT_FDCWD},
        path::Dentry,
        registry::{FsProperties, FsType},
        utils::{
            DirentCounter, DirentVisitor, FallocMode, FileSystem, FsFlags, Inode, InodeMode,
            InodeType, IoctlCmd, Metadata, MknodType, SuperBlock, XattrName, XattrNamespace,
            XattrSetFlags, NAME_MAX, XATTR_VALUE_MAX_LEN,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::vmo::Vmo,
};

const OVERLAY_FS_MAGIC: u64 = 0x794C7630;

/// An `OverlayFS` is a union pseudo file system employed to merge
/// upper and lower directories that potentially comes from different
/// file systems, into a single, unified view at a designated mount point.
pub struct OverlayFS {
    /// The writable upper layer.
    upper: OverlayUpper,
    /// The read-only lower layer.
    lower: OverlayLower,
    /// The work directory.
    work: OverlayWork,
    /// Configuration settings.
    config: OverlayConfig,
    /// Super block.
    sb: OverlaySB,
    /// Unique inode number generator.
    next_ino: AtomicU64,
    /// Weak self reference.
    self_: Weak<OverlayFS>,
}

/// The mutable upper layer of an `OverlayFS`.
struct OverlayUpper {
    dentry: Dentry,
}

/// The immutable lower layer of an `OverlayFS`.
/// A lower layer may contain multiple `Dentry`s with different mount points.
struct OverlayLower {
    /// Layered dentries from top to bottom.
    dentries: Vec<Dentry>,
    // TODO: Support data-only lower layers.
}

/// The work directory. Must reside in
/// the same file system as the upper layer.
struct OverlayWork {
    dentry: Dentry,
    // TODO: Align the work directory's behavior with Linux.
}

/// Provides an unified inode abstraction for its user, internal it
/// manages the layered regular inodes.
struct OverlayInode {
    /// The unique inode number.
    ino: u64,
    /// The inode type.
    type_: InodeType,
    /// The name parameter in `Inode::create` issued by the parent.
    /// This field is used to build hierarchical upper inodes.
    /// The lock is intended to implement `rename`.
    name_upon_creation: SpinLock<String>,
    /// The parent inode. `None` for root inode.
    parent: Option<Arc<OverlayInode>>,
    /// The mutable upper regular inode.
    upper: Mutex<Option<Arc<dyn Inode>>>,
    /// Whether the upper inode is an opaque directory.
    upper_is_opaque: bool,
    /// The immutable lower layered regular inodes.
    lowers: Vec<Arc<dyn Inode>>,
    /// Weak fs reference.
    fs: Weak<OverlayFS>,
    /// Weak self reference.
    self_: Weak<OverlayInode>,
}

impl OverlayFS {
    /// Creates a new OverlayFS instance.
    ///
    /// # Arguments
    /// * `upper` - The upper directory (writable layer)
    /// * `lower` - Vector of lower directories (read-only layers, in priority order)
    /// * `work` - The work directory (must be empty and on same filesystem as upper)
    ///
    /// # Returns
    /// An `Arc<OverlayFS>` on success, or an error if validation fails.
    ///
    /// # Errors
    /// * `EINVAL` - If work and upper are on different filesystems
    /// * `EINVAL` - If work is not empty
    pub fn new(upper: Dentry, lower: Vec<Dentry>, work: Dentry) -> Result<Arc<Self>> {
        Self::validate_work_and_upper(&work, &upper)?;
        Self::validate_work_empty(&work)?;

        Ok(Arc::new_cyclic(|weak| Self {
            upper: OverlayUpper { dentry: upper },
            lower: OverlayLower { dentries: lower },
            work: OverlayWork { dentry: work },
            config: OverlayConfig::default(),
            sb: OverlaySB,
            next_ino: AtomicU64::new(0),
            self_: weak.clone(),
        }))
    }

    /// Validates that work is on the same filesystem as upper.
    fn validate_work_and_upper(work: &Dentry, upper: &Dentry) -> Result<()> {
        if !Arc::ptr_eq(upper.mount_node(), work.mount_node()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "workdir and upperdir must reside under the same mount"
            );
        }
        Ok(())
    }

    /// Validates that work is empty.
    fn validate_work_empty(work: &Dentry) -> Result<()> {
        let mut counter = DirentCounter::new();
        let _ = work.inode().readdir_at(0, &mut counter);
        if counter.count() > 0 {
            return_errno_with_message!(Errno::EINVAL, "workdir must be empty");
        }
        Ok(())
    }
}

impl FileSystem for OverlayFS {
    /// Utilizes the layered directory entries to build the root inode.
    fn root_inode(&self) -> Arc<dyn Inode> {
        let fs = self.fs();
        let upper_inode = fs.upper.dentry.inode().clone();
        let ino = upper_inode.ino();
        Arc::new_cyclic(|weak| OverlayInode {
            ino,
            type_: InodeType::Dir,
            name_upon_creation: SpinLock::new(String::from("")),
            parent: None,
            upper: Mutex::new(Some(upper_inode)),
            upper_is_opaque: false,
            lowers: fs
                .lower
                .dentries
                .iter()
                .map(|dentry| dentry.inode())
                .cloned()
                .collect(),
            fs: self.self_.clone(),
            self_: weak.clone(),
        })
    }

    fn sync(&self) -> Result<()> {
        // TODO: Issue sync to all upper inodes.
        Ok(())
    }

    fn sb(&self) -> SuperBlock {
        // TODO: Fill the super block with valid field values.
        SuperBlock::new(OVERLAY_FS_MAGIC, BLOCK_SIZE, NAME_MAX)
    }

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
    }
}

impl OverlayFS {
    fn fs(&self) -> Arc<OverlayFS> {
        self.self_.upgrade().unwrap()
    }

    /// Allocates a new unique inode number.
    fn alloc_ino(&self) -> u64 {
        self.next_ino.fetch_add(1, Ordering::Relaxed)
    }
}

// Inode APIs
impl OverlayInode {
    /// Lookups the target child `OverlayInode`. If the child is not present in cache,
    /// it will be built from the layered lookups within the lower layers.
    pub fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        match self.lookup_inner(name) {
            Ok(Some(inode)) => Ok(inode),
            Ok(None) => Err(Error::new(Errno::ENOENT)),
            Err(e) => Err(e),
        }
    }

    /// Creates a new non-exist child `OverlayInode` in the upper layer.
    /// If the parent directories do not exist, they will be created recursively in the upper layer.
    pub fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        // TODO: Hold the upper lock from here to avoid race condition
        let is_whiteout = match self.lookup_inner(name) {
            Ok(Some(_)) => return_errno!(Errno::EEXIST),
            Ok(None) => true,
            Err(e) => {
                if e.error() != Errno::ENOENT {
                    return Err(e);
                } else {
                    false
                }
            }
        };

        if !is_whiteout {
            self.build_upper_recursively_if_needed()?;
        }

        // Protect the whole create operation
        let upper_guard = self.upper.lock();
        let upper = upper_guard.as_ref().unwrap();

        let mut upper_is_opaque = false;
        if is_whiteout {
            // Delete the whiteout file first then create the new file
            // or the new opaque directory.
            upper.unlink(&whiteout_name(name))?;

            if type_ == InodeType::Dir {
                upper_is_opaque = true;
            }
        }

        let new_upper = upper.create(name, type_, mode)?;
        if upper_is_opaque {
            new_upper.set_xattr(
                XattrName::try_from_full_name(OPAQUE_DIR_XATTR_NAME).unwrap(),
                &mut VmReader::from(WHITEOUT_AND_OPAQUE_XATTR_VALUE.as_slice()).to_fallible(),
                XattrSetFlags::CREATE_ONLY,
            )?;
        }

        let new_child = Arc::new_cyclic(|weak| OverlayInode {
            ino: new_upper.ino(),
            type_,
            name_upon_creation: SpinLock::new(String::from(name)),
            parent: Some(self.self_.upgrade().unwrap()),
            upper: Mutex::new(Some(new_upper)),
            upper_is_opaque,
            lowers: Vec::new(),
            fs: self.fs.clone(),
            self_: weak.clone(),
        });
        Ok(new_child)
    }

    /// Writes data to the target inode, if it resides in the lower layer,
    /// it will be copied up to the upper layer.
    /// The corresponding parent directories will be created also if they do not exist.
    pub fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }
        let upper = self.build_upper_recursively_if_needed()?;
        upper.write_at(offset, reader)
    }

    pub fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }
        let upper = self.build_upper_recursively_if_needed()?;
        upper.write_direct_at(offset, reader)
    }

    pub fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }
        self.get_top_valid_inode().read_at(offset, writer)
    }

    pub fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }
        self.get_top_valid_inode().read_direct_at(offset, writer)
    }

    /// Returns the children objects in a unified view.
    /// The object from the upper layer with the same name will mask the lower ones.
    pub fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let overlay_dir_visitor = self.readdir_inner(offset)?;

        for (offset, (name, ino, type_)) in overlay_dir_visitor.as_merged_view() {
            visitor.visit(name, *ino, *type_, *offset)?;
        }

        Ok(overlay_dir_visitor.cur_offset())
    }

    /// Deletes the target file by creating a "whiteout" file from the upper layer.
    /// The corresponding parent directories will be created also if they do not exist.
    pub fn unlink(&self, name: &str) -> Result<()> {
        // TODO: Hold the upper lock from here to avoid race condition
        let inode = self.lookup(name)?;
        let target = inode.downcast_ref::<OverlayInode>().unwrap();
        if target.type_() == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }

        let mut upper_guard = self.upper.lock();
        if upper_guard.is_none() {
            drop(upper_guard);
            self.build_upper_recursively_if_needed()?;
            upper_guard = self.upper.lock();
        }

        let upper = upper_guard.as_ref().unwrap();
        let target_has_valid_lower = target.has_valid_lower();
        if target.has_valid_upper() {
            upper.unlink(name)?;
        } else {
            assert!(target_has_valid_lower);
        }

        if target_has_valid_lower {
            let whiteout = upper.create(
                &whiteout_name(name),
                InodeType::File,
                InodeMode::from_bits_truncate(0o644),
            )?;
            // FIXME: Align the whiteout xattr behavior with Linux
            whiteout.set_xattr(
                XattrName::try_from_full_name(WHITEOUT_XATTR_NAME).unwrap(),
                &mut VmReader::from(WHITEOUT_AND_OPAQUE_XATTR_VALUE.as_slice()).to_fallible(),
                XattrSetFlags::CREATE_ONLY,
            )?;
        }

        Ok(())
    }

    /// Deletes the target directory by creating an "opaque" directory from the upper layer.
    /// The corresponding parent directories will be created also if they do not exist.
    pub fn rmdir(&self, name: &str) -> Result<()> {
        // TODO: Hold the upper lock from here to avoid race condition
        let inode = self.lookup(name)?;
        let target = inode.downcast_ref::<OverlayInode>().unwrap();
        if target.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        self.build_upper_recursively_if_needed()?;
        let upper_guard = self.upper.lock();
        let upper = upper_guard.as_ref().unwrap();

        let visitor = target.readdir_inner(0)?;
        if visitor.visited_files() > 0 {
            return_errno!(Errno::ENOTEMPTY);
        }

        // Delete all the whiteout files if necessary
        if visitor.contains_whiteout() {
            let target_upper = target.upper().unwrap();

            let mut target_visitor = Vec::<String>::new();
            target_upper.readdir_at(0, &mut target_visitor)?;

            for whiteout in target_visitor.iter().skip(2) {
                assert!(whiteout.starts_with(WHITEOUT_PREFIX));
                target_upper.unlink(whiteout)?;
            }
        }

        upper.rmdir(name)?;

        let whiteout = upper.create(
            &whiteout_name(name),
            InodeType::File,
            InodeMode::from_bits_truncate(0o644),
        )?;
        // FIXME: Align the whiteout xattr behavior with Linux
        whiteout.set_xattr(
            XattrName::try_from_full_name(WHITEOUT_XATTR_NAME).unwrap(),
            &mut VmReader::from(WHITEOUT_AND_OPAQUE_XATTR_VALUE.as_slice()).to_fallible(),
            XattrSetFlags::CREATE_ONLY,
        )?;

        Ok(())
    }

    pub fn fs(&self) -> Arc<dyn FileSystem> {
        self.overlay_fs() as _
    }

    pub fn resize(&self, new_size: usize) -> Result<()> {
        if self.type_ != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "not regular file");
        }

        if self.get_top_valid_inode().size() == new_size {
            return Ok(());
        }

        let upper = self.build_upper_recursively_if_needed()?;
        upper.resize(new_size)
    }

    pub fn metadata(&self) -> Metadata {
        let mut metadata = self.get_top_valid_inode().metadata();
        metadata.ino = self.ino;
        metadata
    }

    pub fn ino(&self) -> u64 {
        self.ino
    }

    pub fn type_(&self) -> InodeType {
        self.type_
    }

    pub fn page_cache(&self) -> Option<Vmo<Full>> {
        let _ = self.get_top_valid_inode().page_cache()?;
        // Do copy-up for the potential memory mapping operations
        let upper = self.build_upper_recursively_if_needed().unwrap();
        upper.page_cache()
    }

    pub fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        if self.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "not mknod on a dir");
        }
        let upper = self.build_upper_recursively_if_needed()?;
        upper.mknod(name, mode, type_)
    }

    pub fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        if self.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        let upper = self.build_upper_recursively_if_needed()?;
        upper.link(old, name)
    }

    pub fn read_link(&self) -> Result<String> {
        if self.type_ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }
        self.get_top_valid_inode().read_link()
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        if self.type_ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }
        let upper = self.build_upper_recursively_if_needed()?;
        upper.write_link(target)
    }

    pub fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        // TODO: Support the rename operation based on the `redirect_mode` feature,
        // rename the upper only may unexpectedly reveal the lower inodes.
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "rename is not supported in current overlayfs"
        );
    }

    pub fn sync_all(&self) -> Result<()> {
        self.upper().map_or(Ok(()), |upper| upper.sync_all())
    }

    pub fn sync_data(&self) -> Result<()> {
        self.upper().map_or(Ok(()), |upper| upper.sync_data())
    }

    pub fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        self.upper().map_or_else(
            || {
                Err(Error::with_message(
                    Errno::EOPNOTSUPP,
                    "ioctl is not supported",
                ))
            },
            |upper| upper.ioctl(cmd, arg),
        )
    }
}

#[inherit_methods(from = "self.get_top_valid_inode()")]
impl OverlayInode {
    pub fn size(&self) -> usize; // TODO: Calculate the right size for directory
    pub fn mode(&self) -> Result<InodeMode>;
    pub fn owner(&self) -> Result<Uid>;
    pub fn group(&self) -> Result<Gid>;
    pub fn atime(&self) -> Duration;
    pub fn mtime(&self) -> Duration;
    pub fn ctime(&self) -> Duration;
    pub fn as_device(&self) -> Option<Arc<dyn Device>>;
    pub fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize>;
    pub fn list_xattr(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize>;
}

#[inherit_methods(from = "self.build_upper_recursively_if_needed()?")]
impl OverlayInode {
    // TODO: Support the `metacopy` feature for efficiency
    pub fn set_mode(&self, mode: InodeMode) -> Result<()>;
    pub fn set_owner(&self, uid: Uid) -> Result<()>;
    pub fn set_group(&self, gid: Gid) -> Result<()>;
    pub fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()>;
}

#[inherit_methods(from = "self.build_upper_recursively_if_needed().unwrap()")]
impl OverlayInode {
    pub fn set_atime(&self, time: Duration);
    pub fn set_mtime(&self, time: Duration);
    pub fn set_ctime(&self, time: Duration);
}

impl OverlayInode {
    // Returns the top valid inode who must exist.
    fn get_top_valid_inode(&self) -> Arc<dyn Inode> {
        if let Some(upper) = self.upper() {
            return upper;
        }

        self.get_top_valid_lower_inode().cloned().unwrap()
    }

    /// Returns the top valid lower inode.
    fn get_top_valid_lower_inode(&self) -> Option<&Arc<dyn Inode>> {
        if !self.has_valid_lower() {
            return None;
        }

        // Note that the whiteout or opaque check is performed in `lookup` and `create`,
        // the only two places where an `OverlayInode` can be created.
        // So a lower inode can never be a whiteout file or opaque directory.
        Some(&self.lowers[0])
    }

    fn has_valid_lower(&self) -> bool {
        !self.lowers.is_empty()
    }

    fn has_valid_upper(&self) -> bool {
        self.upper.lock().is_some()
    }

    /// Returns the upper inode if it exists.
    fn upper(&self) -> Option<Arc<dyn Inode>> {
        // Note that the whiteout or opaque check is performed in `lookup` and `create`,
        // the only two places where an `OverlayInode` can be created.
        // So the upper inode can never be a whiteout file or opaque directory.
        self.upper.lock().clone()
    }

    fn num_lowers(&self) -> usize {
        self.lowers.len()
    }

    fn is_opaque_dir(&self) -> bool {
        self.type_ == InodeType::Dir && self.upper_is_opaque
    }

    fn name_upon_creation(&self) -> String {
        self.name_upon_creation.lock().clone()
    }

    fn overlay_fs(&self) -> Arc<OverlayFS> {
        self.fs.upgrade().unwrap()
    }

    /// Lookups the target regular inodes in a layered manner then
    /// builds the corresponding `OverlayInode`.
    /// The whiteout and opaque checks are performed here only.
    fn lookup_inner(&self, name: &str) -> Result<Option<Arc<dyn Inode>>> {
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let mut type_ = None;
        let mut upper_is_opaque = false;
        let mut upper_is_not_dir = false;

        let upper_child = if let Some(upper) = self.upper.lock().as_ref() {
            // First check whiteout then opaque
            if upper.lookup(&whiteout_name(name)).is_ok() {
                // Provide whiteout information for `create`
                return Ok(None);
            }

            match upper.lookup(name) {
                Ok(child) => {
                    let child_type = child.type_();
                    if child_type == InodeType::Dir {
                        upper_is_opaque = is_opaque_dir(&child)?;
                    } else {
                        upper_is_not_dir = true;
                    }

                    let _ = type_.insert(child_type);
                    Some(child)
                }
                Err(e) => {
                    if e.error() == Errno::ENOENT {
                        None
                    } else {
                        return Err(e);
                    }
                }
            }
        } else {
            None
        };

        let lower_children = if upper_is_opaque || upper_is_not_dir {
            vec![]
        } else {
            let mut children = Vec::new();
            for lower in &self.lowers {
                if lower.lookup(&whiteout_name(name)).is_ok() {
                    break;
                }

                if let Ok(child) = lower.lookup(name) {
                    let child_type = child.type_();
                    let is_child_opaque = child_type == InodeType::Dir && is_opaque_dir(&child)?;

                    if upper_child.is_none() && children.is_empty() {
                        children.push(child);
                        let _ = type_.insert(child_type);
                    } else {
                        let type_ = type_.unwrap();
                        if type_ != InodeType::Dir || type_ != child_type {
                            break;
                        } else {
                            children.push(child);
                        }
                    }

                    if is_child_opaque {
                        break;
                    }
                }
            }
            children
        };

        if upper_child.is_none() && lower_children.is_empty() {
            return_errno!(Errno::ENOENT);
        }

        let ino = if let Some(upper) = &upper_child {
            UniqueNoGenerator::gen_unique_ino(0 as LayerIdx, upper.ino())?
        } else {
            UniqueNoGenerator::gen_unique_ino(1 as LayerIdx, lower_children[0].ino())?
        };
        let child_ovl_inode = Arc::new_cyclic(|weak| OverlayInode {
            ino,
            type_: type_.unwrap(),
            name_upon_creation: SpinLock::new(String::from(name)),
            parent: Some(self.self_.upgrade().unwrap()),
            upper: Mutex::new(upper_child),
            upper_is_opaque,
            lowers: lower_children,
            fs: self.fs.clone(),
            self_: weak.clone(),
        });

        Ok(Some(child_ovl_inode))
    }

    fn readdir_inner(&self, offset: usize) -> Result<OverlayDirVisitor> {
        let mut overlay_visitor = OverlayDirVisitor::new();
        let (mut layer_idx, fs_offset) = UniqueNoGenerator::parse_unique_offset(offset);

        // Process all the potential whiteout files before `layer_idx`
        if layer_idx > 0 {
            overlay_visitor.set_whiteout_only_mode();

            let upper = self.upper();
            for idx in 0..=layer_idx {
                let cur_inode = if idx == 0 {
                    upper.as_ref()
                } else {
                    self.lowers.get(idx as usize - 1)
                };

                if let Some(cur_inode) = cur_inode {
                    cur_inode.readdir_at(0, &mut overlay_visitor)?;
                }
            }

            overlay_visitor.unset_whiteout_only_mode();
        }

        // Process all files from `layer_idx` and `fs_offset`
        if layer_idx == 0 {
            if let Some(upper) = self.upper() {
                debug_assert!(upper.type_() == InodeType::Dir);
                upper.readdir_at(fs_offset, &mut overlay_visitor)?;
            }

            layer_idx += 1;
            overlay_visitor.set_cur_layer(layer_idx);
        }

        if !self.is_opaque_dir() && layer_idx > 0 && layer_idx as usize <= self.lowers.len() {
            // TODO: Figure out how to check the opaque directories within lower layers.
            let first_lower = &self.lowers[layer_idx as usize - 1];
            first_lower.readdir_at(fs_offset, &mut overlay_visitor)?;

            layer_idx += 1;
            overlay_visitor.set_cur_layer(layer_idx);

            for lower in self.lowers.iter().skip(layer_idx as usize - 1) {
                lower.readdir_at(0, &mut overlay_visitor)?;

                layer_idx += 1;
                overlay_visitor.set_cur_layer(layer_idx);
            }
        }

        Ok(overlay_visitor)
    }

    fn build_upper_recursively_if_needed(&self) -> Result<Arc<dyn Inode>> {
        let mut upper_guard = self.upper.lock();
        if let Some(upper) = upper_guard.as_ref() {
            return Ok(upper.clone());
        }

        debug_assert!(self.parent.is_some());
        // FIXME: Should we hold every upper locks from lower to upper
        // for such a long period?
        let parent_upper = self
            .parent
            .as_ref()
            .unwrap()
            .build_upper_recursively_if_needed()?;

        let mode = self.get_top_valid_lower_inode().unwrap().mode()?;
        let new_upper = parent_upper.create(&self.name_upon_creation(), self.type_, mode)?;

        // There must exist a valid lower inode if the upper is missing
        assert!(!self.lowers.is_empty());
        self.do_copy_up(&new_upper)?;

        let _ = upper_guard.insert(new_upper.clone());
        Ok(new_upper)
    }

    /// Do the "copy-up" operation for the given upper inode.
    fn do_copy_up(&self, upper_inode: &Arc<dyn Inode>) -> Result<()> {
        if self.lowers.is_empty() {
            return Ok(());
        }
        let Some(lower_inode) = self.get_top_valid_lower_inode() else {
            return Ok(());
        };

        let upper_type = upper_inode.type_();
        if upper_type != lower_inode.type_() {
            return Ok(());
        }

        // First copy the metadata, then the data, finally the xattr
        Self::copy_up_metadata(lower_inode, upper_inode)?;

        if upper_type == InodeType::File {
            Self::copy_up_data(lower_inode, upper_inode)?;
        }

        Self::copy_up_xattr(lower_inode, upper_inode)?;
        Ok(())
    }

    fn copy_up_metadata(lower: &Arc<dyn Inode>, upper: &Arc<dyn Inode>) -> Result<()> {
        // TODO: We lack an efficient whole metadata copy API.

        // The mode is copied up upon creation.
        upper.set_owner(lower.owner()?)?;
        upper.set_group(lower.group()?)?;
        upper.set_atime(lower.atime());
        upper.set_mtime(lower.mtime());
        upper.set_ctime(lower.ctime());
        Ok(())
    }

    fn copy_up_data(lower: &Arc<dyn Inode>, upper: &Arc<dyn Inode>) -> Result<()> {
        debug_assert!(lower.type_() == InodeType::File && upper.type_() == InodeType::File);
        if lower.size() == 0 {
            return Ok(());
        }

        // TODO: Find a way to cut this copy, like just copy chunks of data from two page caches directly.
        let lower_size = lower.size();
        let data_buf = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(lower_size.align_up(BLOCK_SIZE) / BLOCK_SIZE)?;

        let mut writer = data_buf.writer().to_fallible();
        let read_len = lower.read_at(0, &mut writer)?;

        let mut reader = data_buf.reader().to_fallible();
        let _ = upper.write_at(0, reader.limit(read_len))?;
        Ok(())
    }

    fn copy_up_xattr(lower: &Arc<dyn Inode>, upper: &Arc<dyn Inode>) -> Result<()> {
        debug_assert!(lower.type_() == upper.type_());

        let list_len = lower.list_xattr(
            XattrNamespace::Trusted,
            &mut VmWriter::from([].as_mut_slice()).to_fallible(),
        )?;
        if list_len == 0 {
            return Ok(());
        }
        let mut list = vec![0u8; list_len];
        lower.list_xattr(
            XattrNamespace::Trusted,
            &mut VmWriter::from(list.as_mut_slice()).to_fallible(),
        )?;

        let value_buf = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(XATTR_VALUE_MAX_LEN / PAGE_SIZE)?;
        for name in list
            .split(|&byte| byte == 0)
            .map(|slice| String::from_utf8_lossy(slice))
        {
            if name.is_empty() {
                break;
            }
            let value_len = lower.get_xattr(
                XattrName::try_from_full_name(name.as_ref()).unwrap(),
                &mut value_buf.writer().to_fallible(),
            )?;
            let mut value_reader = value_buf.reader().to_fallible();
            upper.set_xattr(
                XattrName::try_from_full_name(name.as_ref()).unwrap(),
                value_reader.limit(value_len),
                XattrSetFlags::CREATE_ONLY,
            )?;
        }
        Ok(())
    }
}

const WHITEOUT_XATTR_NAME: &str = "trusted.overlay.whiteout";
const OPAQUE_DIR_XATTR_NAME: &str = "trusted.overlay.opaque";
const WHITEOUT_AND_OPAQUE_XATTR_VALUE: [u8; 1] = [121u8]; // "y", represents the xattr is set

const WHITEOUT_PREFIX: &str = ".wh.";
const WHITEOUT_PREFIX_SIZE: usize = WHITEOUT_PREFIX.len();

fn whiteout_name(name: &str) -> String {
    format!("{}{}", WHITEOUT_PREFIX, name)
}

fn is_opaque_dir(inode: &Arc<dyn Inode>) -> Result<bool> {
    assert_eq!(inode.type_(), InodeType::Dir);

    let name = XattrName::try_from_full_name(OPAQUE_DIR_XATTR_NAME).unwrap();
    let mut value = [0u8];
    if let Err(e) = inode.get_xattr(
        name,
        &mut VmWriter::from(value.as_mut_slice()).to_fallible(),
    ) {
        match e.error() {
            Errno::E2BIG | Errno::ENODATA | Errno::EOPNOTSUPP | Errno::ERANGE => {
                return Ok(false);
            }
            _ => return Err(e),
        }
    };
    Ok(value == WHITEOUT_AND_OPAQUE_XATTR_VALUE)
}

#[inherit_methods(from = "self")]
impl Inode for OverlayInode {
    fn size(&self) -> usize;
    fn resize(&self, new_size: usize) -> Result<()>;
    fn metadata(&self) -> Metadata;
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
    fn page_cache(&self) -> Option<Vmo<Full>>;
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;
    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;
    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize>;
    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize>;
    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>>;
    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>>;
    fn as_device(&self) -> Option<Arc<dyn Device>>;
    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize>;
    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()>;
    fn unlink(&self, name: &str) -> Result<()>;
    fn rmdir(&self, name: &str) -> Result<()>;
    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>>;
    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()>;
    fn read_link(&self) -> Result<String>;
    fn write_link(&self, target: &str) -> Result<()>;
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;
    fn sync_all(&self) -> Result<()>;
    fn sync_data(&self) -> Result<()>;
    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()>;
    fn fs(&self) -> Arc<dyn FileSystem>;
    fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()>;
    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize>;
    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize>;
    fn remove_xattr(&self, name: XattrName) -> Result<()>;
}

/// The index of the layer of an `OverlayFS`.
type LayerIdx = u8; // Currently only support 256 layers.

/// A visitor used by `OverlayFS` that merges the objects
/// from the upper layer and the lower layer.
struct OverlayDirVisitor {
    dir_map: BTreeMap<usize, (String, u64, InodeType)>,
    dir_set: HashSet<String>,
    whiteout_set: HashSet<String>,
    visited_files: usize,
    cur_layer: LayerIdx,
    whiteout_only_mode: bool,
}

impl DirentVisitor for OverlayDirVisitor {
    fn visit(&mut self, name: &str, fs_ino: u64, type_: InodeType, fs_offset: usize) -> Result<()> {
        if self.whiteout_only_mode && name.starts_with(WHITEOUT_PREFIX) {
            self.dir_set
                .insert(name[WHITEOUT_PREFIX_SIZE..].to_string());
            return Ok(());
        }

        let unique_offset = UniqueNoGenerator::gen_unique_offset(self.cur_layer, fs_offset)?;
        let unique_ino = UniqueNoGenerator::gen_unique_ino(self.cur_layer, fs_ino)?;

        if self.dir_set.contains(name) || self.whiteout_set.contains(name) {
            return Ok(());
        }
        if name.starts_with(WHITEOUT_PREFIX) {
            self.whiteout_set
                .insert(name[WHITEOUT_PREFIX_SIZE..].to_string());
            return Ok(());
        }

        let name = name.to_string();
        self.dir_set.insert(name.clone());

        if name != "." && name != ".." {
            self.visited_files += 1;
        }

        debug_assert!(!self.dir_map.contains_key(&unique_offset));
        let _ = self
            .dir_map
            .insert(unique_offset, (name, unique_ino, type_));
        Ok(())
    }
}

impl OverlayDirVisitor {
    pub fn new() -> Self {
        Self {
            dir_map: BTreeMap::new(),
            dir_set: HashSet::new(),
            whiteout_set: HashSet::new(),
            visited_files: 0,
            cur_layer: 0,
            whiteout_only_mode: false,
        }
    }

    /// Returns the merged view of the directory.
    pub fn as_merged_view(&self) -> impl Iterator<Item = (&usize, &(String, u64, InodeType))> + '_ {
        self.dir_map.iter()
    }

    pub fn visited_files(&self) -> usize {
        self.visited_files
    }

    pub fn contains_whiteout(&self) -> bool {
        !self.whiteout_set.is_empty()
    }

    pub fn cur_offset(&self) -> usize {
        self.dir_map
            .last_key_value()
            .map(|(off, _)| *off)
            .unwrap_or(0)
    }

    fn set_cur_layer(&mut self, layer: LayerIdx) {
        self.cur_layer = layer;
    }

    fn set_whiteout_only_mode(&mut self) {
        self.whiteout_only_mode = true;
    }

    fn unset_whiteout_only_mode(&mut self) {
        self.whiteout_only_mode = false;
    }
}

struct UniqueNoGenerator;

// Unique offset and ino layout: `| LayerIdx (8 bits) | Real fs offset or ino (56 bits) |`
impl UniqueNoGenerator {
    const NUM_HIGHER_BITS: usize = 8;
    const NUM_LOWER_BITS: usize = 56;
    const HIGHER_MASK: usize = 0xFF00_0000_0000_0000;
    const LOWER_MASK: usize = 0x00FF_FFFF_FFFF_FFFF;

    pub fn gen_unique_offset(layer_idx: LayerIdx, fs_offset: usize) -> Result<usize> {
        if fs_offset & Self::HIGHER_MASK != 0 {
            return_errno_with_message!(Errno::EOVERFLOW, "fs offset overflow");
        }
        Ok(((layer_idx as usize) << Self::NUM_LOWER_BITS) | fs_offset)
    }

    // XXX: Linux uses embedded fsid for unique ino. Should we follow the technique?
    pub fn gen_unique_ino(layer_idx: LayerIdx, fs_ino: u64) -> Result<u64> {
        if (fs_ino as usize) & Self::HIGHER_MASK != 0 {
            return_errno_with_message!(Errno::EOVERFLOW, "fs ino overflow");
        }
        Ok(((layer_idx as u64) << Self::NUM_LOWER_BITS) | fs_ino)
    }

    pub fn parse_unique_offset(offset: usize) -> (LayerIdx, usize) {
        let layer = (offset >> Self::NUM_LOWER_BITS) as LayerIdx;
        let offset = offset & Self::LOWER_MASK;
        (layer, offset)
    }
}

/// Holds various mode settings and feature toggles.
// TODO: Try to support these features and make them configurable.
// Check https://github.com/torvalds/linux/blob/master/Documentation/filesystems/overlayfs.rst for more.
#[derive(Default)]
pub struct OverlayConfig {
    default_permissions: bool,
    redirect_mode: u8,
    verity_mode: u8,
    index: u8,
    uuid: u32,
    nfs_export: bool,
    xino: u64,
    metacopy: bool,
    userxattr: bool,
    ovl_volatile: bool,
}

// TODO: Complete the super block struct.
struct OverlaySB;

pub(super) struct OverlayFsType;

impl FsType for OverlayFsType {
    fn name(&self) -> &'static str {
        "overlay"
    }

    fn create(
        &self,
        args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
        ctx: &Context,
    ) -> Result<Arc<dyn FileSystem>> {
        let mut lower = Vec::new();
        let mut upper = "";
        let mut work = "";

        let args = args.ok_or(Error::new(Errno::EINVAL))?;
        let args = args.to_string_lossy();
        let entries = args.split(',');

        for entry in entries {
            let mut parts = entry.split('=');
            match (parts.next(), parts.next()) {
                // Handle lowerdir, split by ':'
                (Some("upperdir"), Some(path)) => {
                    if path.is_empty() {
                        return_errno_with_message!(Errno::ENOENT, "upperdir is empty");
                    }
                    upper = path;
                }
                (Some("lowerdir"), Some(paths)) => {
                    for path in paths.split(':') {
                        if path.is_empty() {
                            return_errno_with_message!(Errno::ENOENT, "lowerdir is empty");
                        }
                        lower.push(path);
                    }
                }
                (Some("workdir"), Some(path)) => {
                    if path.is_empty() {
                        return_errno_with_message!(Errno::ENOENT, "workdir is empty");
                    }
                    work = path;
                }
                _ => (),
            }
        }

        let fs = ctx.posix_thread.fs().resolver().read();

        let upper = fs.lookup(&FsPath::new(AT_FDCWD, upper)?)?;
        let lower = lower
            .iter()
            .map(|lower| fs.lookup(&FsPath::new(AT_FDCWD, lower).unwrap()).unwrap())
            .collect();
        let work = fs.lookup(&FsPath::new(AT_FDCWD, work)?)?;

        OverlayFS::new(upper, lower, work).map(|fs| fs as _)
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysBranchNode>> {
        None
    }
}

// TODO: Enrich the tests to cover more cases.
#[cfg(ktest)]
mod tests {
    use ostd::{mm::VmIo, prelude::ktest};

    use super::*;
    use crate::fs::{path::MountNode, ramfs::RamFS};

    fn create_overlay_fs() -> Arc<dyn FileSystem> {
        crate::time::clocks::init_for_ktest();

        let mode = InodeMode::all();
        let upper = {
            let root_mount = MountNode::new_root(RamFS::new());
            Dentry::new_fs_root(root_mount)
        };
        let lower = {
            let r1 = MountNode::new_root(RamFS::new());
            let r2 = MountNode::new_root(RamFS::new());

            let l1 = Dentry::new_fs_root(r1);
            l1.new_fs_child("f1", InodeType::File, mode).unwrap();
            let d1 = l1.new_fs_child("d1", InodeType::Dir, mode).unwrap();
            d1.new_fs_child("f11", InodeType::File, mode).unwrap();

            let l2 = Dentry::new_fs_root(r2);
            let f2 = l2.new_fs_child("f2", InodeType::File, mode).unwrap();
            let f2_inode = f2.inode();
            f2_inode
                .write_at(0, &mut VmReader::from([8u8; 4].as_slice()).to_fallible())
                .unwrap();
            f2_inode.set_group(Gid::new(77)).unwrap();
            f2_inode
                .set_xattr(
                    XattrName::try_from_full_name("trusted.f2_xattr_name").unwrap(),
                    &mut VmReader::from("f2_xattr_value".as_bytes()).to_fallible(),
                    XattrSetFlags::CREATE_ONLY,
                )
                .unwrap();
            let d1 = l2.new_fs_child("d1", InodeType::Dir, mode).unwrap();
            d1.new_fs_child("f11", InodeType::File, mode).unwrap();
            d1.new_fs_child("f12", InodeType::File, mode).unwrap();

            vec![l1, l2]
        };
        let work = upper.clone();

        let fs = OverlayFS::new(upper, lower, work).unwrap();
        assert_eq!(fs.sb().magic, OVERLAY_FS_MAGIC);
        fs
    }

    #[ktest]
    fn work_and_upper_should_be_in_same_mount() {
        crate::time::clocks::init_for_ktest();

        let upper = Dentry::new_fs_root(MountNode::new_root(RamFS::new()));
        let lower = vec![Dentry::new_fs_root(MountNode::new_root(RamFS::new()))];
        let work = Dentry::new_fs_root(MountNode::new_root(RamFS::new()));

        let Err(e) = OverlayFS::new(upper, lower, work) else {
            panic!("OverlayFS::new should fail when work and upper are not in the same mount");
        };
        assert_eq!(e.error(), Errno::EINVAL);
    }

    #[ktest]
    fn work_should_be_empty() {
        crate::time::clocks::init_for_ktest();

        let mode = InodeMode::all();
        let upper = {
            let root = Dentry::new_fs_root(MountNode::new_root(RamFS::new()));
            root.new_fs_child("file", InodeType::File, mode).unwrap();
            root
        };
        let lower = vec![Dentry::new_fs_root(MountNode::new_root(RamFS::new()))];
        let work = upper.clone();

        let Err(e) = OverlayFS::new(upper, lower, work) else {
            panic!("OverlayFS::new should fail when work is not empty");
        };
        assert_eq!(e.error(), Errno::EINVAL);
    }

    #[ktest]
    fn obscured_multi_layers() {
        crate::time::clocks::init_for_ktest();

        let mode = InodeMode::all();
        let root = Dentry::new_fs_root(MountNode::new_root(RamFS::new()));
        let upper = {
            let dir = root.new_fs_child("upper", InodeType::Dir, mode).unwrap();
            dir.new_fs_child("f1", InodeType::File, mode).unwrap();
            dir.new_fs_child(".wh.f2", InodeType::File, mode).unwrap();
            dir.new_fs_child("d1", InodeType::Dir, mode).unwrap();
            dir.new_fs_child("d2", InodeType::Dir, mode).unwrap();
            dir.new_fs_child(".wh.d3", InodeType::Dir, mode).unwrap();
            dir
        };
        let lower = {
            let l1 = {
                let r1 = Dentry::new_fs_root(MountNode::new_root(RamFS::new()));
                r1.new_fs_child("f1", InodeType::Dir, mode).unwrap();
                r1.new_fs_child("f2", InodeType::File, mode).unwrap();
                let d1 = r1.new_fs_child("d1", InodeType::Dir, mode).unwrap();
                d1.set_xattr(
                    XattrName::try_from_full_name(OPAQUE_DIR_XATTR_NAME).unwrap(),
                    &mut VmReader::from(WHITEOUT_AND_OPAQUE_XATTR_VALUE.as_slice()).to_fallible(),
                    XattrSetFlags::CREATE_ONLY,
                )
                .unwrap();
                r1.new_fs_child("d2", InodeType::File, mode).unwrap();
                r1.new_fs_child("d3", InodeType::Dir, mode).unwrap();
                r1
            };
            let l2 = {
                let r2 = Dentry::new_fs_root(MountNode::new_root(RamFS::new()));
                r2.new_fs_child("f1", InodeType::File, mode).unwrap();
                r2.new_fs_child("d1", InodeType::Dir, mode).unwrap();
                r2.new_fs_child("d2", InodeType::Dir, mode).unwrap();
                r2.new_fs_child("d4", InodeType::Dir, mode).unwrap();
                r2
            };
            vec![l1, l2]
        };
        let work = root.new_fs_child("work", InodeType::Dir, mode).unwrap();

        let fs = OverlayFS::new(upper, lower, work).unwrap();
        let root = fs.root_inode();

        let f1 = root.lookup("f1").unwrap();
        assert_eq!(f1.type_(), InodeType::File);
        let f1_inode = f1.downcast_ref::<OverlayInode>().unwrap();
        assert!(f1_inode.has_valid_upper() && !f1_inode.has_valid_lower());

        let e = root.lookup("f2").expect_err("");
        assert_eq!(e.error(), Errno::ENOENT);

        let d1 = root.lookup("d1").unwrap();
        assert_eq!(d1.type_(), InodeType::Dir);
        let d1_inode = d1.downcast_ref::<OverlayInode>().unwrap();
        assert!(d1_inode.has_valid_upper() && d1_inode.num_lowers() == 1);

        let d2 = root.lookup("d2").unwrap();
        assert_eq!(d2.type_(), InodeType::Dir);
        let d2_inode = d2.downcast_ref::<OverlayInode>().unwrap();
        assert!(d2_inode.has_valid_upper() && !d2_inode.has_valid_lower());

        let e = root.lookup("d3").expect_err("");
        assert_eq!(e.error(), Errno::ENOENT);

        let d4 = root.lookup("d4").unwrap();
        assert_eq!(d4.type_(), InodeType::Dir);
        let d4_inode = d4.downcast_ref::<OverlayInode>().unwrap();
        assert!(!d4_inode.has_valid_upper() && d4_inode.num_lowers() == 1);
    }

    #[ktest]
    fn read_files_and_dirs() {
        let fs = create_overlay_fs();
        let root = fs.root_inode();

        let f1 = root.lookup("f1").unwrap();
        assert_eq!(f1.type_(), InodeType::File);

        let mut data = [0u8; 4];
        let f2 = root.lookup("f2").unwrap();
        f2.read_at(0, &mut VmWriter::from(data.as_mut_slice()).to_fallible())
            .unwrap();
        assert_eq!(data, [8u8; 4]);

        let d1 = root.lookup("d1").unwrap();
        assert_eq!(d1.type_(), InodeType::Dir);
        let mut d1_fnames = Vec::<String>::new();
        // No assumption on the return value
        let _ = d1.readdir_at(0, &mut d1_fnames).unwrap();
        assert_eq!(d1_fnames, [".", "..", "f11", "f12"]);
    }

    #[ktest]
    fn whiteout_file() {
        let fs = create_overlay_fs();
        let root = fs.root_inode();
        let mode = InodeMode::all();

        let Err(e) = root.create("f1", InodeType::File, mode) else {
            panic!();
        };
        assert_eq!(e.error(), Errno::EEXIST);
        root.unlink("f1").unwrap();

        root.create("f1", InodeType::File, mode).unwrap();
    }

    #[ktest]
    fn opaque_dir() {
        let fs = create_overlay_fs();
        let root = fs.root_inode();
        let mode = InodeMode::all();

        let Err(e) = root.create("d1", InodeType::Dir, mode) else {
            panic!();
        };
        assert_eq!(e.error(), Errno::EEXIST);

        let d1 = root.lookup("d1").unwrap();
        d1.unlink("f11").unwrap();
        d1.unlink("f12").unwrap();

        root.rmdir("d1").unwrap();
        let d1 = root.create("d1", InodeType::Dir, mode).unwrap();
        d1.create("f11", InodeType::File, mode).unwrap();
    }

    #[ktest]
    fn copy_up() {
        let fs = create_overlay_fs();
        let root = fs.root_inode();

        let f2 = root.lookup("f2").unwrap();

        f2.write_bytes_at(2, &[9u8; 2]).unwrap();
        let mut data = [0u8; 4];
        f2.read_bytes_at(0, data.as_mut_slice()).unwrap();
        assert_eq!(data, [8u8, 8, 9, 9]);

        assert_eq!(f2.group().unwrap(), Gid::new(77));

        let mut xattr_value = [0u8; 14];
        f2.get_xattr(
            XattrName::try_from_full_name("trusted.f2_xattr_name").unwrap(),
            &mut VmWriter::from(xattr_value.as_mut_slice()).to_fallible(),
        )
        .unwrap();
        assert_eq!(xattr_value.as_slice(), "f2_xattr_value".as_bytes());
    }

    #[ktest]
    fn basic_operations() {
        let fs = create_overlay_fs();
        let root = fs.root_inode();
        let mode = InodeMode::all();

        let f1 = root.lookup("f1").unwrap();
        assert_eq!(f1.size(), 0);
        f1.resize(PAGE_SIZE).unwrap();
        f1.page_cache().unwrap().write_val(0, &3u8).unwrap();
        f1.set_atime(Duration::default());
        f1.sync_data().unwrap();
        let mut data = [0u8; 1];
        f1.read_at(0, &mut VmWriter::from(data.as_mut_slice()).to_fallible())
            .unwrap();
        assert_eq!(data, [3u8; 1]);

        let d1 = root.lookup("d1").unwrap();
        d1.set_mode(mode).unwrap();
        assert_ne!(f1.ino(), d1.ino());
        d1.mknod("dev", mode, MknodType::NamedPipeNode).unwrap();

        let link = d1.create("link", InodeType::SymLink, mode).unwrap();
        let link_str = "link_to_somewhere";
        link.write_link(link_str).unwrap();
        assert_eq!(link.read_link().unwrap(), link_str.to_string());
    }
}
