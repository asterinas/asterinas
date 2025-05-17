// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::any::Any;

use bitflags::bitflags;
use ostd::{mm::VmWriter, sync::RwLock};

use crate::{fs::path::Dentry, prelude::*};

pub mod inotify;
pub use inotify::{InotifyFile, InotifyFlags, InotifyMask};

use super::utils::{Inode, InodeType};

#[derive(Debug)]
pub struct FsnotifyCommon {
    fsnotify_mask: u32,
    fsnotify_marks: RwLock<Vec<Arc<dyn FsnotifyMark>>>,
}

impl FsnotifyCommon {
    pub fn new() -> Self {
        Self {
            fsnotify_mask: 0,
            fsnotify_marks: RwLock::new(Vec::new()),
        }
    }

    pub fn fsnotify_mask(&self) -> u32 {
        self.fsnotify_mask
    }

    pub fn add_fsnotify_mark(&self, mark: Arc<dyn FsnotifyMark>, _add_flags: u32) {
        self.fsnotify_marks.write().push(mark);
    }

    pub fn remove_fsnotify_mark(&self, mark: &Arc<dyn FsnotifyMark>) {
        self.fsnotify_marks
            .write()
            .retain(|m| !Arc::ptr_eq(m, mark));
    }

    pub fn remove_fsnotify_marks(&self) {
        while let Some(mark) = self.fsnotify_marks.write().pop() {
            // Get the group reference first
            let group = mark.fsnotify_group().clone();
            // Now we can safely call free_mark without holding the mark's lock
            group.free_mark(&mark);
        }
    }

    pub fn send_fsnotify(&self, mask: u32, name: String) {
        // Traverse all the marks and send the fsnotify event to the group.
        let marks = self.fsnotify_marks.read();
        for mark in marks.iter() {
            // We should check the mask if group is interested in the event.
            let group = mark.fsnotify_group();
            group.send_event(mark, mask, name.clone());
        }
    }

    pub fn find_fsnotify_mark(
        &self,
        fsnotify_group: &Arc<dyn FsnotifyGroup>,
    ) -> Option<Arc<dyn FsnotifyMark>> {
        self.fsnotify_marks
            .read()
            .iter()
            .find(|mark| Arc::ptr_eq(&mark.fsnotify_group(), fsnotify_group))
            .cloned()
    }
}

/// A group is a "thing" that wants to receive notification about filesystem
/// events.  The notifications is a list of events that are sent to the group.
/// The marks is a list of marks that are attached to the group.
pub trait FsnotifyGroup: Any + Send + Sync + Debug {
    fn send_event(&self, mark: &Arc<dyn FsnotifyMark>, mask: u32, name: String);
    fn pop_event(&self) -> Option<Arc<dyn FsnotifyEvent>>;
    fn get_all_event_size(&self) -> usize;
    fn free_mark(&self, mark: &Arc<dyn FsnotifyMark>);
}

pub trait FsnotifyEvent: Any + Send + Sync + Debug {
    fn copy_to_user(&self, writer: &mut VmWriter) -> Result<usize>;
    fn get_size(&self) -> usize;
}

/// A mark is simply an object attached to an in core inode which allows an
/// fsnotify listener to indicate they are either no longer interested in events
/// of a type matching mask or only interested in those events.
/// These are flushed when an inode is evicted from core and may be flushed
/// when the inode is modified (as seen by fsnotify_access).  Some fsnotify
/// users (such as dnotify) will flush these when the open fd is closed and not
/// at inode eviction or modification.
pub trait FsnotifyMark: Any + Send + Sync + Debug {
    /// Group this mark is for
    fn fsnotify_group(&self) -> Arc<dyn FsnotifyGroup>;
    fn update_mark(&self, mask: u32) -> Result<u32>;
}

impl dyn FsnotifyMark {
    pub fn downcast_ref<T: FsnotifyMark>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

bitflags! {
    pub struct FsnotifyMarkFlags: u32 {
        // General fsnotify mark flags
        const FSNOTIFY_MARK_FLAG_ALIVE               = 0x0001;
        const FSNOTIFY_MARK_FLAG_ATTACHED            = 0x0002;

        // Inotify mark flags
        const FSNOTIFY_MARK_FLAG_EXCL_UNLINK         = 0x0010;
        const FSNOTIFY_MARK_FLAG_IN_ONESHOT          = 0x0020;

        // Fanotify mark flags
        const FSNOTIFY_MARK_FLAG_IGNORED_SURV_MODIFY = 0x0100;
        const FSNOTIFY_MARK_FLAG_NO_IREF             = 0x0200;
        const FSNOTIFY_MARK_FLAG_HAS_IGNORE_FLAGS    = 0x0400;
    }
}

bitflags! {
    pub struct FsnotifyFlags: u32 {
        const FS_ACCESS          = 0x00000001; // File was accessed
        const FS_MODIFY          = 0x00000002; // File was modified
        const FS_ATTRIB          = 0x00000004; // Metadata changed
        const FS_CLOSE_WRITE     = 0x00000008; // Writable file was closed
        const FS_CLOSE_NOWRITE   = 0x00000010; // Unwritable file closed
        const FS_OPEN            = 0x00000020; // File was opened
        const FS_MOVED_FROM      = 0x00000040; // File was moved from X
        const FS_MOVED_TO        = 0x00000080; // File was moved to Y
        const FS_CREATE          = 0x00000100; // Subfile was created
        const FS_DELETE          = 0x00000200; // Subfile was deleted
        const FS_DELETE_SELF     = 0x00000400; // Self was deleted
        const FS_MOVE_SELF       = 0x00000800; // Self was moved
        const FS_OPEN_EXEC       = 0x00001000; // File was opened for exec
        const FS_UNMOUNT         = 0x00002000; // Inode on umount fs
        const FS_Q_OVERFLOW      = 0x00004000; // Event queued overflowed
        const FS_ERROR           = 0x00008000; // Filesystem Error (fanotify)
        const FS_IN_IGNORED      = 0x00008000; // Last inotify event here
        const FS_OPEN_PERM       = 0x00010000; // Open event in a permission hook
        const FS_ACCESS_PERM     = 0x00020000; // Access event in a permissions hook
        const FS_OPEN_EXEC_PERM  = 0x00040000; // Open/exec event in a permission hook
        const FS_EVENT_ON_CHILD  = 0x08000000; // Set on inode mark that cares about things that happen to its children.
        const FS_RENAME          = 0x10000000; // File was renamed
        const FS_DN_MULTISHOT    = 0x20000000; // dnotify multishot
        const FS_ISDIR           = 0x40000000; // Event occurred against dir
    }
}

/// File was read
pub fn fsnotify_access(dentry: &Dentry) -> Result<()> {
    fsnotify_parent(dentry, FsnotifyFlags::FS_ACCESS, dentry.effective_name())?;
    if dentry.inode().type_() == InodeType::Dir {
        fsnotify(
            dentry.inode(),
            FsnotifyFlags::FS_ACCESS | FsnotifyFlags::FS_ISDIR,
            String::new(),
        )?;
    } else {
        fsnotify(dentry.inode(), FsnotifyFlags::FS_ACCESS, String::new())?;
    }
    Ok(())
}

/// File was modified
pub fn fsnotify_modify(dentry: &Dentry) -> Result<()> {
    fsnotify_parent(dentry, FsnotifyFlags::FS_MODIFY, dentry.effective_name())?;
    fsnotify(dentry.inode(), FsnotifyFlags::FS_MODIFY, String::new())?;
    Ok(())
}

/// Dentry was unlinked and unhashed
pub fn fsnotify_delete(
    dir_inode: &Arc<dyn Inode>,
    inode: &Arc<dyn Inode>,
    name: String,
) -> Result<()> {
    if inode.type_() == InodeType::Dir {
        fsnotify(
            dir_inode,
            FsnotifyFlags::FS_DELETE | FsnotifyFlags::FS_ISDIR,
            name,
        )
    } else {
        fsnotify(dir_inode, FsnotifyFlags::FS_DELETE, name)
    }
}

/// Inode's link count changed
pub fn fsnotify_link_count(inode: &Arc<dyn Inode>) -> Result<()> {
    fsnotify(inode, FsnotifyFlags::FS_ATTRIB, String::new())
}

/// Called when an inode is removed, specifically when its link count reaches 0
pub fn fsnotify_inode_removed(inode: &Arc<dyn Inode>) -> Result<()> {
    fsnotify(inode, FsnotifyFlags::FS_DELETE_SELF, String::new())
}

/// Inode was linked
pub fn fsnotify_link(
    dir_inode: &Arc<dyn Inode>,
    inode: &Arc<dyn Inode>,
    name: String,
) -> Result<()> {
    fsnotify_link_count(inode)?;
    fsnotify(dir_inode, FsnotifyFlags::FS_CREATE, name)
}

/// Directory was created
pub fn fsnotify_mkdir(dentry: &Dentry, name: String) -> Result<()> {
    fsnotify(
        dentry.inode(),
        FsnotifyFlags::FS_CREATE | FsnotifyFlags::FS_ISDIR,
        name,
    )
}

/// File was created
pub fn fsnotify_create(dentry: &Dentry, name: String) -> Result<()> {
    fsnotify(dentry.inode(), FsnotifyFlags::FS_CREATE, name)
}

/// File was opened
pub fn fsnotify_open(dentry: &Dentry) -> Result<()> {
    fsnotify_parent(dentry, FsnotifyFlags::FS_OPEN, dentry.effective_name())?;
    fsnotify(dentry.inode(), FsnotifyFlags::FS_OPEN, String::new())?;
    Ok(())
}

/// File was closed
pub fn fsnotify_close(dentry: &Dentry) -> Result<()> {
    // TODO: check file's mode is contain FMODE_WRITE
    fsnotify_parent(
        dentry,
        FsnotifyFlags::FS_CLOSE_WRITE,
        dentry.effective_name(),
    )?;
    fsnotify(dentry.inode(), FsnotifyFlags::FS_CLOSE_WRITE, String::new())?;
    Ok(())
}

/// File's attributes changed
pub fn fsnotify_attr_change(dentry: &Dentry) -> Result<()> {
    fsnotify_parent(dentry, FsnotifyFlags::FS_ATTRIB, dentry.effective_name())?;
    fsnotify(dentry.inode(), FsnotifyFlags::FS_ATTRIB, String::new())?;
    Ok(())
}

/// Notify this dentry's parent about a child's events with child name info
/// if parent is watching or if inode/sb/mount are interested in events with
/// parent and name info.
///
/// Notify only the child without name info if parent is not watching and
/// inode/sb/mount are not interested in events with parent and name info.
fn fsnotify_parent(dentry: &Dentry, data_type: FsnotifyFlags, name: String) -> Result<()> {
    let parent = dentry.parent();
    if let Some(parent) = parent {
        fsnotify(&parent.inode(), data_type, name)?;
    }
    Ok(())
}

/// This is the main call to fsnotify.
///
/// The VFS calls into hook specific functions in `fs/notify/`.
/// Those functions then in turn call here.  Here will call out to all of the
/// registered fsnotify_group.  Those groups can then use the notification event
/// in whatever means they feel necessary.
fn fsnotify(inode: &Arc<dyn Inode>, data_type: FsnotifyFlags, name: String) -> Result<()> {
    inode.send_fsnotify(data_type.bits(), name);
    Ok(())
}
