// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::any::Any;

use bitflags::bitflags;
use ostd::sync::RwLock;

use crate::{
    fs::{file_handle::FileLike, notify::inotify::InotifyFile, path::Path, AccessMode},
    prelude::*,
};

pub mod inotify;

use super::utils::{Inode, InodeType};

/// FsnotifyPublisher maintains a list of fsnotify subscribers.
/// It provides methods to add, remove, and notify subscribers about filesystem events.
/// FsnotifyPublisher is used by inodes to manage their subscribers.
/// When a filesystem event occurs, the inode publishes the event on its FsnotifyPublisher.
/// The publisher then notifies all registered subscribers about the event.
pub struct FsnotifyPublisher {
    /// List of fsnotify subscribers.
    fsnotify_subscribers: RwLock<Vec<Arc<dyn FsnotifySubscriber>>>,
}

impl Debug for FsnotifyPublisher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let subscribers = self.fsnotify_subscribers.read();
        write!(
            f,
            "FsnotifyPublisher: num_subscribers: {}",
            subscribers.len()
        )?;
        Ok(())
    }
}

impl Default for FsnotifyPublisher {
    fn default() -> Self {
        Self::new()
    }
}

impl FsnotifyPublisher {
    pub fn new() -> Self {
        Self {
            fsnotify_subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Add a subscriber to this publisher.
    pub fn add_subscriber(&self, subscriber: Arc<dyn FsnotifySubscriber>) {
        let mut subscribers = self.fsnotify_subscribers.write();
        subscribers.push(subscriber);
    }

    /// Remove a subscriber from this publisher.
    pub fn remove_subscriber(&self, subscriber: &Arc<dyn FsnotifySubscriber>) {
        let mut subscribers = self.fsnotify_subscribers.write();
        subscribers.retain(|m| !Arc::ptr_eq(m, subscriber));
    }

    /// Remove all subscribers from this publisher.
    /// This is typically called when the publisher is being destroyed.
    pub fn remove_all_subscribers(&self) -> Result<()> {
        let mut subscribers = self.fsnotify_subscribers.write();
        // Notify all subscribers about IN_IGNORED event before removing them.
        for subscriber in subscribers.iter() {
            subscriber.deliver_event(FsnotifyEvents::IN_IGNORED, None)?;
        }
        // Clear all subscribers.
        subscribers.clear();
        Ok(())
    }

    /// The publisher notifies all its subscribers about the event.
    pub fn publish_event(&self, events: FsnotifyEvents, name: Option<String>) -> Result<()> {
        // Traverse all the marks and send the fsnotify event to the subscribers.
        let subscribers = self.fsnotify_subscribers.read();
        for subscriber in subscribers.iter() {
            // We should check the mask if group is interested in the event.
            subscriber.deliver_event(events, name.clone())?;
        }
        Ok(())
    }

    /// Find inotify subscriber by inotify file.
    pub fn find_inotify_subscriber(
        &self,
        inotify_file: &Arc<InotifyFile>,
    ) -> Option<Arc<dyn FsnotifySubscriber>> {
        let subscribers = self.fsnotify_subscribers.read();
        for subscriber in subscribers.iter() {
            if let Some(inotify_subscriber) =
                (subscriber.as_ref() as &dyn Any).downcast_ref::<inotify::InotifySubscriber>()
            {
                if Arc::ptr_eq(&inotify_subscriber.inotify_file(), inotify_file) {
                    return Some(subscriber.clone());
                }
            }
        }
        None
    }
}

/// A subscriber is an object subscribed to filesystem events on an FsnotifyPublisher.
/// The publisher will notify the subscriber when an event occurs that matches the subscriber's interest.
/// Interests are specified via a mask. The publisher is attached to an inode.
/// When the filesystem event occurs in the inode such as read, write, modify, delete .etc,
/// the publisher notifies all its subscribers about the event.
pub trait FsnotifySubscriber: Any + Send + Sync {
    /// The subscriber deliver notification of filesystem events from the publisher to specific file.
    /// In inotify file, it delivers the event to the inotify file, then the inotify file queues the event to its event queue.
    /// The subscriber implementation should filter events based on its interest mask.
    fn deliver_event(&self, events: FsnotifyEvents, name: Option<String>) -> Result<()>;
}

bitflags! {
    /// FsnotifyEvents is representing the events that occurred.
    /// These events are used to notify subscribers about specific filesystem actions.
    /// In specific, notify subscribers can use interest to filter events they care about.
    pub struct FsnotifyEvents: u32 {
        const ACCESS          = 0x00000001; // File was accessed
        const MODIFY          = 0x00000002; // File was modified
        const ATTRIB          = 0x00000004; // Metadata changed
        const CLOSE_WRITE     = 0x00000008; // Writable file was closed
        const CLOSE_NOWRITE   = 0x00000010; // Unwritable file closed
        const OPEN            = 0x00000020; // File was opened
        const MOVED_FROM      = 0x00000040; // File was moved from X
        const MOVED_TO        = 0x00000080; // File was moved to Y
        const CREATE          = 0x00000100; // Subfile was created
        const DELETE          = 0x00000200; // Subfile was deleted
        const DELETE_SELF     = 0x00000400; // Self was deleted
        const MOVE_SELF       = 0x00000800; // Self was moved
        const OPEN_EXEC       = 0x00001000; // File was opened for exec
        const UNMOUNT         = 0x00002000; // Inode on umount fs
        const Q_OVERFLOW      = 0x00004000; // Event queued overflowed
        const ERROR           = 0x00008000; // Filesystem Error (fanotify)
        const IN_IGNORED      = 0x00008000; // Last inotify event here
        const OPEN_PERM       = 0x00010000; // Open event in a permission hook
        const ACCESS_PERM     = 0x00020000; // Access event in a permissions hook
        const OPEN_EXEC_PERM  = 0x00040000; // Open/exec event in a permission hook
        const EVENT_ON_CHILD  = 0x08000000; // Set on inode mark that cares about things that happen to its children.
        const RENAME          = 0x10000000; // File was renamed
        const DN_MULTISHOT    = 0x20000000; // dnotify multishot
        const ISDIR           = 0x40000000; // Event occurred against dir
    }
}

/// File was read.
/// path is the Path of the file that was read.
pub fn on_access(path: &Path) -> Result<()> {
    fsnotify_parent(path, FsnotifyEvents::ACCESS, path.effective_name())?;
    let mut event = FsnotifyEvents::ACCESS;
    if path.inode().type_() == InodeType::Dir {
        event |= FsnotifyEvents::ISDIR;
    }
    fsnotify(path.inode(), event, None)?;
    Ok(())
}

/// File was modified.
/// path is the Path of the file that was modified.
pub fn on_modify(path: &Path) -> Result<()> {
    fsnotify_parent(path, FsnotifyEvents::MODIFY, path.effective_name())?;
    fsnotify(path.inode(), FsnotifyEvents::MODIFY, None)?;
    Ok(())
}

/// Path was unlinked and unhashed.
/// dir_inode is the Inode of the directory that the file was unlinked from.
/// inode is the Inode of the file that was unlinked.
/// name is the name of the file that was unlinked.
pub fn on_delete(dir_inode: &Arc<dyn Inode>, inode: &Arc<dyn Inode>, name: String) -> Result<()> {
    if inode.type_() == InodeType::Dir {
        fsnotify(
            dir_inode,
            FsnotifyEvents::DELETE | FsnotifyEvents::ISDIR,
            Some(name),
        )
    } else {
        fsnotify(dir_inode, FsnotifyEvents::DELETE, Some(name))
    }
}

/// Inode's link count changed.
/// inode is the Inode of the file that was linked.
pub fn on_link_count(inode: &Arc<dyn Inode>) -> Result<()> {
    fsnotify(inode, FsnotifyEvents::ATTRIB, None)
}

/// Called when an inode is removed, specifically when its link count reaches 0.
/// inode is the Inode of the file that was removed.
pub fn on_inode_removed(inode: &Arc<dyn Inode>) -> Result<()> {
    fsnotify(inode, FsnotifyEvents::DELETE_SELF, None)
}

/// Inode was linked.
/// dir_inode is the Inode of the parent directory that was linked.
/// inode is the Inode of the file that was linked.
/// name is the name of the file that was linked
pub fn on_link(dir_inode: &Arc<dyn Inode>, inode: &Arc<dyn Inode>, name: String) -> Result<()> {
    on_link_count(inode)?;
    fsnotify(dir_inode, FsnotifyEvents::CREATE, Some(name))
}

/// Directory was created.
/// path is the Path of the parent directory that was created.
/// name is the name of the directory that was created.
pub fn on_mkdir(path: &Path, name: String) -> Result<()> {
    fsnotify(
        path.inode(),
        FsnotifyEvents::CREATE | FsnotifyEvents::ISDIR,
        Some(name),
    )
}

/// File was created.
/// path is the Path of the parent directory that was created.
/// name is the name of the file that was created.
pub fn on_create(path: &Path, name: String) -> Result<()> {
    fsnotify(path.inode(), FsnotifyEvents::CREATE, Some(name))
}

/// File was opened.
/// path is the Path of the file that was opened.
pub fn on_open(path: &Path) -> Result<()> {
    let mut event = FsnotifyEvents::OPEN;
    if path.inode().type_() == InodeType::Dir {
        event |= FsnotifyEvents::ISDIR;
    }
    fsnotify_parent(path, event, path.effective_name())?;
    fsnotify(path.inode(), event, None)?;
    Ok(())
}

/// File was closed.
/// path is the Path of the file that was closed.
pub fn on_close(file: Arc<dyn FileLike>) -> Result<()> {
    // Some file is not supported dentry, such as epoll file,
    // TODO: Add anonymous inode support.
    if let Some(path) = file.path() {
        let flag = match file.access_mode() {
            AccessMode::O_RDONLY => FsnotifyEvents::CLOSE_NOWRITE,
            _ => FsnotifyEvents::CLOSE_WRITE,
        };
        let mut event = flag;
        if path.inode().type_() == InodeType::Dir {
            event |= FsnotifyEvents::ISDIR;
        }
        fsnotify_parent(path, event, path.effective_name())?;
        fsnotify(path.inode(), event, None)?;
    }
    Ok(())
}

/// File's attributes changed.
/// path is the Path of the file that was modified.
pub fn on_attr_change(path: &Path) -> Result<()> {
    fsnotify_parent(path, FsnotifyEvents::ATTRIB, path.effective_name())?;
    fsnotify(path.inode(), FsnotifyEvents::ATTRIB, None)?;
    Ok(())
}

/// Notify this path's parent about a child's events with child name info
/// if parent is watching or if inode are interested in events with
/// parent and name info.
///
/// Notify only the child without name info if parent is not watching and
/// inode are not interested in events with parent and name info.
fn fsnotify_parent(path: &Path, events: FsnotifyEvents, name: String) -> Result<()> {
    let parent = path.effective_parent();
    if let Some(parent) = parent {
        fsnotify(parent.inode(), events, Some(name))?;
    }
    Ok(())
}

/// This is the main call to fsnotify.
///
/// The VFS calls into hook specific functions in `fs/notify/`.
/// Those functions then in turn call here. Here will call publisher to
/// send events to all registered subscribers.
fn fsnotify(inode: &Arc<dyn Inode>, events: FsnotifyEvents, name: Option<String>) -> Result<()> {
    inode.fsnotify_publisher().publish_event(events, name)?;
    Ok(())
}
