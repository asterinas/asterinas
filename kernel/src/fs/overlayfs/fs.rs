// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use aster_block::BLOCK_SIZE;
use aster_rights::Full;
use hashbrown::HashSet;
use inherit_methods_macro::inherit_methods;
use spin::Once;

use crate::{
    fs::{
        device::Device,
        path::Dentry,
        utils::{
            DirentVisitor, FallocMode, FileSystem, FsFlags, Inode, InodeMode, InodeType, IoctlCmd,
            Metadata, MknodType, SuperBlock, XattrName, XattrNamespace, XattrSetFlags, NAME_MAX,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::vmo::Vmo,
};

const OVERLAY_FS_MAGIC: u64 = 0x794C7630;

/// An `OverlayFS` is a union file system that
/// combines upper, lower, and work directories.
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

/// The mutable upper layer.
struct OverlayUpper {
    dentry: Dentry,
}

/// Global upper path set for exclusive overlay upper mount check.
static OVL_EXCLUSIVE_UPPERS: Once<SpinLock<HashSet<String>>> = Once::new();

/// The immutable lower layer.
struct OverlayLower {
    /// Layered dentries from top to bottom.
    dentries: Vec<Dentry>,
    // TODO: Support data-only lower layers
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
    /// The name of the inode in the parent directory.
    name: String,
    /// The parent inode. `None` for root inode.
    parent: Option<Arc<OverlayInode>>,
    /// The mutable upper regular inode.
    upper: Mutex<Option<Arc<dyn Inode>>>,
    /// The immutable lower layered regular inodes.
    lowers: Vec<Arc<dyn Inode>>,
    /// Weak fs reference.
    fs: Weak<OverlayFS>,
    /// Weak self reference.
    self_: Weak<OverlayInode>,
}

impl OverlayFS {
    pub fn new(upper: Dentry, lower: Vec<Dentry>, work: Dentry) -> Result<Arc<Self>> {
        let upper_path_not_occupied = OVL_EXCLUSIVE_UPPERS
            .call_once(|| SpinLock::new(HashSet::new()))
            .lock()
            .insert(upper.abs_path());
        if !upper_path_not_occupied {
            return_errno_with_message!(
                Errno::EINVAL,
                "the upper path of overlayfs must be exclusive"
            );
        }

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
}

impl FileSystem for OverlayFS {
    /// Utilizes the layered directory entries to build the root inode.
    fn root_inode(&self) -> Arc<dyn Inode> {
        Arc::new_cyclic(|weak| {
            let fs = self.fs();
            OverlayInode {
                ino: self.alloc_ino(),
                type_: InodeType::Dir,
                name: fs.upper.dentry.effective_name(),
                parent: None,
                upper: Mutex::new(Some(fs.upper.dentry.inode().clone())),
                lowers: fs
                    .lower
                    .dentries
                    .iter()
                    .map(|dentry| dentry.inode())
                    .cloned()
                    .collect(),
                fs: self.self_.clone(),
                self_: weak.clone(),
            }
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

        // TODO: Since the negative lookup is heavy, we assume
        // the existence check is done in the VFS layer.
        let opaque_or_whiteout = match self.lookup_inner(name) {
            Ok(Some(_)) => return_errno!(Errno::EEXIST),
            Ok(None) => true,
            Err(_) => false,
        };

        if !opaque_or_whiteout {
            self.build_upper_recursively_if_needed()?;
        }

        // Protect the whole create operation
        let upper_guard = self.upper.lock();
        let upper = upper_guard.as_ref().unwrap();

        if opaque_or_whiteout && type_ == InodeType::File {
            let _ = upper.unlink(name);
        } else if opaque_or_whiteout && type_ == InodeType::Dir {
            let _ = upper.rmdir(name);
        }

        let new_upper = upper.create(name, type_, mode)?;

        let new_child = Arc::new_cyclic(|weak| OverlayInode {
            ino: self.overlay_fs().alloc_ino(),
            type_,
            name: String::from(name),
            parent: Some(self.self_.upgrade().unwrap()),
            upper: Mutex::new(Some(new_upper)),
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

        let mut overlay_dir_visitor = OverlayDirMergedVisitor::new();
        if let Some(upper) = self.upper() {
            debug_assert!(upper.type_() == InodeType::Dir);
            // The upper cannot be an opaque.
            upper.readdir_at(offset, &mut overlay_dir_visitor)?;
        }

        for lower in self.lowers.iter() {
            debug_assert!(lower.type_() == InodeType::Dir);
            // The lower cannot be an opaque.
            lower.readdir_at(offset, &mut overlay_dir_visitor)?;
        }

        let mut cnt = 0;
        for (offset, (name, ino, type_)) in overlay_dir_visitor.as_merged_view() {
            visitor.visit(name, *ino, *type_, *offset)?;
            cnt += 1;
        }
        Ok(cnt)
    }

    /// Deletes the target file by creating a "whiteout" file from the upper layer.
    /// The corresponding parent directories will be created also if they do not exist.
    pub fn unlink(&self, name: &str) -> Result<()> {
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
        // The target file cannot be a whiteout.
        if let Err(e) = upper.unlink(name) {
            if e.error() != Errno::ENOENT {
                return Err(e);
            }
        }

        let whiteout = upper.create(name, InodeType::File, InodeMode::from_bits_truncate(0o644))?;
        whiteout.set_xattr(
            XattrName::try_from_full_name(WHITEOUT_FILE_XATTR_NAME).unwrap(),
            &mut VmReader::from(WHITEOUT_AND_OPAQUE_XATTR_VALUE.as_slice()).to_fallible(),
            XattrSetFlags::CREATE_ONLY,
        )?;
        Ok(())
    }

    /// Deletes the target directory by creating a "opaque" directory from the upper layer.
    /// The corresponding parent directories will be created also if they do not exist.
    pub fn rmdir(&self, name: &str) -> Result<()> {
        let inode = self.lookup(name)?;
        let target = inode.downcast_ref::<OverlayInode>().unwrap();
        if target.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        // An alternative logic compared to `unlink`. Which one is better?
        self.build_upper_recursively_if_needed()?;
        let upper_guard = self.upper.lock();

        let upper = upper_guard.as_ref().unwrap();
        // The target directory cannot be an opaque.
        if let Err(e) = upper.rmdir(name) {
            if e.error() != Errno::ENOENT {
                return Err(e);
            }
        }

        let opaque = upper.create(name, InodeType::Dir, InodeMode::from_bits_truncate(0o755))?;
        opaque.set_xattr(
            XattrName::try_from_full_name(OPAQUE_DIR_XATTR_NAME).unwrap(),
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

        let upper = if self.get_top_valid_inode().size() != new_size {
            self.build_upper_recursively_if_needed()?
        } else {
            return Ok(());
        };
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
        self.get_top_valid_inode().page_cache()?;
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
        // rename the upper only may unexpectedly reveal the lower indoes
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

    pub fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.build_upper_recursively_if_needed()?
            .fallocate(mode, offset, len)
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

        self.get_top_valid_lower_inode()
            .map(|lower| lower.clone())
            .unwrap()
    }

    /// Returns the top valid lower inode.
    fn get_top_valid_lower_inode(&self) -> Option<&Arc<dyn Inode>> {
        if self.lowers.is_empty() {
            return None;
        }

        // Note that the whiteout or opaque check is performed in `lookup` and `create`,
        // the only two places where an `OverlayInode` can be created.
        // So a lower inode can never be a whiteout file or opaque directory.
        Some(&self.lowers[0])
    }

    /// Returns the upper inode if it exists.
    fn upper(&self) -> Option<Arc<dyn Inode>> {
        // Note that the whiteout or opaque check is performed in `lookup` and `create`,
        // the only two places where an `OverlayInode` can be created.
        // So the upper inode can never be a whiteout file or opaque directory.
        self.upper.lock().clone()
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

        let upper_child = if let Some(upper) = self.upper.lock().as_ref() {
            match upper.lookup(name) {
                Ok(child) => {
                    if is_whiteout_file_or_opaque_dir(&child)? {
                        // Provide more information for `create`
                        return Ok(None);
                    }
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

        let lower_children = {
            let mut children = Vec::new();
            for lower in &self.lowers {
                if let Ok(child) = lower.lookup(name) {
                    if is_whiteout_file_or_opaque_dir(&child)? {
                        break;
                    }
                    children.push(child);
                }
            }
            children
        };

        if upper_child.is_none() && lower_children.is_empty() {
            return_errno!(Errno::ENOENT);
        }

        let type_ = upper_child
            .as_ref()
            .map_or(lower_children[0].type_(), |upper| upper.type_());
        let child_ovl_inode = Arc::new_cyclic(|weak| OverlayInode {
            ino: self.overlay_fs().alloc_ino(),
            type_,
            name: String::from(name),
            parent: Some(self.self_.upgrade().unwrap()),
            upper: Mutex::new(upper_child),
            lowers: lower_children,
            fs: self.fs.clone(),
            self_: weak.clone(),
        });

        Ok(Some(child_ovl_inode))
    }

    fn build_upper_recursively_if_needed(&self) -> Result<Arc<dyn Inode>> {
        let mut upper_guard = self.upper.lock();
        if let Some(upper) = upper_guard.as_ref() {
            return Ok(upper.clone());
        }

        debug_assert!(!self.parent.is_none());
        // FIXME: Should we hold every upper locks from lower to upper
        // for such a long period?
        let parent_upper = self
            .parent
            .as_ref()
            .unwrap()
            .build_upper_recursively_if_needed()?;

        let mode = self.get_top_valid_lower_inode().unwrap().mode()?;
        let new_upper = parent_upper.create(&self.name, self.type_, mode)?;

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

        // First the metadata, then the data, finally the xattr
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

        // TODO: Find a way to cut this copy, like just copy chunks of data from two page caches directly
        let mut data = vec![0; lower.size()];
        lower.read_at(0, &mut VmWriter::from(data.as_mut_slice()).to_fallible())?;
        upper.write_at(0, &mut VmReader::from(data.as_slice()).to_fallible())?;
        Ok(())
    }

    fn copy_up_xattr(lower: &Arc<dyn Inode>, upper: &Arc<dyn Inode>) -> Result<()> {
        // TODO: list + get + set
        Ok(())
    }
}

const WHITEOUT_FILE_XATTR_NAME: &str = "trusted.overlay.whiteout";
const OPAQUE_DIR_XATTR_NAME: &str = "trusted.overlay.opaque";
const WHITEOUT_AND_OPAQUE_XATTR_VALUE: [u8; 1] = [121u8]; // "y", represents the xattr is set

fn is_whiteout_file_or_opaque_dir(inode: &Arc<dyn Inode>) -> Result<bool> {
    let type_ = inode.type_();
    let is_file = type_ == InodeType::File;
    let is_dir = type_ == InodeType::Dir;
    if !is_file && !is_dir {
        return Ok(false);
    }

    let name = XattrName::try_from_full_name(if is_file {
        WHITEOUT_FILE_XATTR_NAME
    } else {
        OPAQUE_DIR_XATTR_NAME
    })
    .unwrap();
    let mut value = [0u8];
    match inode.get_xattr(
        name,
        &mut VmWriter::from(value.as_mut_slice()).to_fallible(),
    ) {
        Err(e) => match e.error() {
            Errno::E2BIG | Errno::ENODATA | Errno::EOPNOTSUPP | Errno::ERANGE => {
                return Ok(false);
            }
            _ => return Err(e),
        },
        Ok(_) => {}
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

/// A visitor used by `OverlayFS` that merges the objects
/// from the upper layer and the lower layer.
struct OverlayDirMergedVisitor {
    dir_map: BTreeMap<usize, (String, u64, InodeType)>,
    dir_set: HashSet<String>,
}

impl DirentVisitor for OverlayDirMergedVisitor {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, offset: usize) -> Result<()> {
        if self.dir_set.contains(name) {
            return Ok(());
        }

        let name = name.to_string();
        self.dir_set.insert(name.clone());

        let offset = self
            .dir_map
            .last_key_value()
            .map(|(off, _)| *off + 1)
            .unwrap_or(offset);
        self.dir_map.insert(offset, (name, ino, type_));
        Ok(())
    }
}

impl OverlayDirMergedVisitor {
    fn new() -> Self {
        Self {
            dir_map: BTreeMap::new(),
            dir_set: HashSet::new(),
        }
    }

    /// Returns the merged view of the directory.
    fn as_merged_view(&self) -> impl Iterator<Item = (&usize, &(String, u64, InodeType))> + '_ {
        self.dir_map.iter()
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

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;
    use crate::fs::{path::MountNode, ramfs::RamFS};

    #[ktest]
    fn ovl_fs() {
        crate::time::clocks::init_for_ktest();

        let mode = InodeMode::all();
        let upper = {
            let root_mount = MountNode::new_root(RamFS::new());
            Dentry::new_fs_root(root_mount)
        };
        let lowers = {
            let r1 = MountNode::new_root(RamFS::new());
            let r2 = MountNode::new_root(RamFS::new());

            let l1 = Dentry::new_fs_root(r1);
            l1.new_fs_child("f1", InodeType::File, mode).unwrap();
            let d1 = l1.new_fs_child("d1", InodeType::Dir, mode).unwrap();
            d1.new_fs_child("f11", InodeType::File, mode).unwrap();

            let l2 = Dentry::new_fs_root(r2);
            let f2 = l2.new_fs_child("f2", InodeType::File, mode).unwrap();
            f2.inode()
                .write_at(0, &mut VmReader::from([8u8; 4].as_slice()).to_fallible())
                .unwrap();
            let d1 = l2.new_fs_child("d1", InodeType::Dir, mode).unwrap();
            d1.new_fs_child("f11", InodeType::File, mode).unwrap();
            d1.new_fs_child("f12", InodeType::File, mode).unwrap();

            vec![l1, l2]
        };

        let fs = Arc::new_cyclic(|weak| OverlayFS {
            upper: OverlayUpper {
                dentry: upper.clone(),
            },
            lower: OverlayLower { dentries: lowers },
            work: OverlayWork {
                dentry: upper.clone(),
            },
            config: OverlayConfig::default(),
            sb: OverlaySB,
            next_ino: AtomicU64::new(0),
            self_: weak.clone(),
        });

        let root = fs.root_inode();

        // 1. Read test
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
        let d1_nfiles = d1.readdir_at(0, &mut d1_fnames).unwrap();
        assert_eq!(d1_nfiles, 4);
        assert_eq!(d1_fnames, [".", "..", "f11", "f12"]);

        // 2. Whiteout test
        let Err(e) = root.create("f1", InodeType::File, mode) else {
            panic!();
        };
        assert_eq!(e.error(), Errno::EEXIST);
        root.unlink("f1").unwrap();

        root.create("f1", InodeType::File, mode).unwrap();

        // 3. Copy up test
        f2.write_bytes_at(2, &[9u8; 2]).unwrap();
        f2.read_bytes_at(0, data.as_mut_slice()).unwrap();
        assert_eq!(data, [8u8, 8, 9, 9]);

        // 4. Opaque dir test
        let Err(e) = root.create("d1", InodeType::Dir, mode) else {
            panic!();
        };
        assert_eq!(e.error(), Errno::EEXIST);
        root.rmdir("d1").unwrap();

        root.create("d1", InodeType::Dir, mode).unwrap();
    }
}
