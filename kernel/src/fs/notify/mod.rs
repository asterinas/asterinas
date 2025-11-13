// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::{
    any::Any,
    sync::atomic::{AtomicU32, Ordering},
};

use bitflags::bitflags;
use ostd::sync::RwLock;

use crate::{
    fs::{file_handle::FileLike, path::Path, utils::AccessMode},
    prelude::*,
};

pub mod inotify;

use super::utils::{Inode, InodeType};

/// Publishes filesystem events to subscribers.
///
/// Each inode has an associated `FsnotifyPublisher` that maintains a list of
/// subscribers interested in filesystem events. When an event occurs, the publisher
/// notifies all subscribers whose interested events match the event.
pub struct FsnotifyPublisher {
    /// List of fsnotify subscribers.
    subscribers: RwLock<Vec<Arc<dyn FsnotifySubscriber>>>,
    /// All interested fsnotify events (aggregated from all subscribers).
    /// Stored as AtomicU32 for lock-free reads, but accessed as FsnotifyEvents.
    all_interested_events: AtomicU32,
}

impl Debug for FsnotifyPublisher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let subscribers = self.subscribers.read();
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
            subscribers: RwLock::new(Vec::new()),
            all_interested_events: AtomicU32::new(0),
        }
    }

    /// Adds a subscriber to this publisher.
    pub fn add_subscriber(&self, subscriber: Arc<dyn FsnotifySubscriber>) {
        let mut subscribers = self.subscribers.write();
        // Update aggregated events with new subscriber's interested events
        self.all_interested_events
            .fetch_or(subscriber.interested_events().bits(), Ordering::Relaxed);
        subscribers.push(subscriber);
    }

    /// Removes a subscriber from this publisher.
    pub fn remove_subscriber(&self, subscriber: &Arc<dyn FsnotifySubscriber>) {
        let mut subscribers = self.subscribers.write();
        subscribers.retain(|m| !Arc::ptr_eq(m, subscriber));
        subscriber.deliver_event(FsnotifyEvents::IN_IGNORED, None);

        // Recalculate aggregated events from remaining subscribers
        self.recalc_interested_events(&subscribers);
    }

    /// Removes all subscribers from this publisher.
    pub fn remove_all_subscribers(&self) -> usize {
        let mut subscribers = self.subscribers.write();
        // Notify all subscribers about IN_IGNORED event before removing them.
        for subscriber in subscribers.iter() {
            subscriber.deliver_event(FsnotifyEvents::IN_IGNORED, None);
        }
        // Return the number of subscribers removed.
        let num_subscribers = subscribers.len();
        // Clear all subscribers.
        subscribers.clear();

        // Clear aggregated events since there are no more subscribers
        self.all_interested_events.store(0, Ordering::Relaxed);

        num_subscribers
    }

    /// Broadcasts an event to all the subscribers of this publisher.
    pub fn publish_event(&self, events: FsnotifyEvents, name: Option<String>) {
        // Fast path: check aggregated events first
        let interested =
            FsnotifyEvents::from_bits_truncate(self.all_interested_events.load(Ordering::Relaxed));
        // If the aggregated events does not intersect with the events, return early.
        if !interested.intersects(events) {
            return;
        }

        // Traverse all the subscribers and send the fsnotify event to them.
        let subscribers = self.subscribers.read();
        for subscriber in subscribers.iter() {
            subscriber.deliver_event(events, name.clone());
        }
    }

    /// Recalculates the aggregated interested events from all subscribers.
    fn recalc_interested_events(&self, subscribers: &[Arc<dyn FsnotifySubscriber>]) {
        let mut new_events = FsnotifyEvents::empty();
        for subscriber in subscribers.iter() {
            new_events |= subscriber.interested_events();
        }
        self.all_interested_events
            .store(new_events.bits(), Ordering::Relaxed);
    }

    /// Updates the aggregated events when a subscriber's interested events change.
    pub fn update_subscriber_events(&self) {
        let subscribers = self.subscribers.read();
        let mut new_events = FsnotifyEvents::empty();
        for subscriber in subscribers.iter() {
            new_events |= subscriber.interested_events();
        }
        self.all_interested_events
            .store(new_events.bits(), Ordering::Relaxed);
    }

    /// Finds a subscriber and applies an action if found.
    ///
    /// The matcher should return `Some(T)` if the subscriber matches and processing
    /// should stop, or `None` to continue searching.
    pub fn find_subscriber_and_process<F, T>(&self, mut matcher: F) -> Option<T>
    where
        F: FnMut(&Arc<dyn FsnotifySubscriber>) -> Option<T>,
    {
        let subscribers = self.subscribers.read();
        for subscriber in subscribers.iter() {
            if let Some(result) = matcher(subscriber) {
                return Some(result);
            }
        }
        None
    }
}

/// Represents a subscriber to filesystem events on an `FsnotifyPublisher`.
///
/// A subscriber receives notifications from a publisher when filesystem events occur
/// that match the subscriber's interested events. The subscriber specifies which events
/// it is interested in using `FsnotifyEvents`, which define the types of events (e.g.,
/// read, write, modify, delete) the subscriber wants to be notified about. When an event
/// occurs, the publisher (attached to an inode) broadcasts it to all subscribers whose
/// interested events match the event type.
pub trait FsnotifySubscriber: Any + Send + Sync {
    /// Delivers a filesystem event notification to the subscriber.
    ///
    /// Invariant: This method must not sleep or perform blocking operations. The publisher
    /// may hold a spin lock when calling this method.
    fn deliver_event(&self, events: FsnotifyEvents, name: Option<String>);
    /// Returns the events that this subscriber is interested in.
    fn interested_events(&self) -> FsnotifyEvents;
}

bitflags! {
    /// Represents filesystem events that have occurred.
    ///
    /// These events are used to notify subscribers about specific filesystem actions.
    /// Subscribers specify which events they are interested in to filter and receive
    /// only the events they care about.
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
        const IN_IGNORED      = 0x00008000; // Last inotify event here (inotify)
        const OPEN_PERM       = 0x00010000; // Open event in a permission hook
        const ACCESS_PERM     = 0x00020000; // Access event in a permissions hook
        const OPEN_EXEC_PERM  = 0x00040000; // Open/exec event in a permission hook
        const EVENT_ON_CHILD  = 0x08000000; // Set on inode mark that cares about things that happen to its children.
        const RENAME          = 0x10000000; // File was renamed
        const DN_MULTISHOT    = 0x20000000; // dnotify multishot
        const ISDIR           = 0x40000000; // Event occurred against dir
    }
}

/// Notifies that a file was accessed.
pub fn on_access(file: &Arc<dyn FileLike>) {
    // TODO: Check fmode flags (FMODE_NONOTIFY, FMODE_NONOTIFY_PERM).
    if let Some(path) = file.path() {
        // Fast path: check if filesystem has any watchers before doing expensive operations
        if !path.inode().fs().fsnotify_info().is_subscribed() {
            return;
        }
        fsnotify_parent(path, FsnotifyEvents::ACCESS, path.effective_name());
    }
}

/// Notifies that a file was modified.
pub fn on_modify(file: &Arc<dyn FileLike>) {
    // TODO: Check fmode flags (FMODE_NONOTIFY, FMODE_NONOTIFY_PERM).
    if let Some(path) = file.path() {
        // Fast path: check if filesystem has any watchers before doing expensive operations
        if !path.inode().fs().fsnotify_info().is_subscribed() {
            return;
        }
        fsnotify_parent(path, FsnotifyEvents::MODIFY, path.effective_name());
    }
}

/// Notifies that a path's content was changed.
pub fn on_change(path: &Path) {
    // Fast path: check if filesystem has any watchers before doing expensive operations
    if !path.inode().fs().fsnotify_info().is_subscribed() {
        return;
    }
    fsnotify_parent(path, FsnotifyEvents::MODIFY, path.effective_name());
}

/// Notifies that a file was deleted from a directory.
pub fn on_delete(dir_inode: &Arc<dyn Inode>, inode: &Arc<dyn Inode>, name: String) {
    // Fast path: check if filesystem has any watchers
    if !dir_inode.fs().fsnotify_info().is_subscribed() {
        return;
    }
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

/// Notifies that an inode's link count changed.
pub fn on_link_count(inode: &Arc<dyn Inode>) {
    if !inode.fs().fsnotify_info().is_subscribed() {
        return;
    }
    fsnotify(inode, FsnotifyEvents::ATTRIB, None);
}

/// Notifies that an inode was removed (link count reached 0).
pub fn on_inode_removed(inode: &Arc<dyn Inode>) {
    if !inode.fs().fsnotify_info().is_subscribed() {
        return;
    }
    fsnotify(inode, FsnotifyEvents::DELETE_SELF, None);
}

/// Notifies that a file was linked to a directory.
pub fn on_link(dir_inode: &Arc<dyn Inode>, inode: &Arc<dyn Inode>, name: String) {
    on_link_count(inode);
    if dir_inode.fs().fsnotify_info().is_subscribed() {
        fsnotify(dir_inode, FsnotifyEvents::CREATE, Some(name))
    }
}

/// Notifies that a directory was created.
pub fn on_mkdir(path: &Path, name: String) {
    if !path.inode().fs().fsnotify_info().is_subscribed() {
        return;
    }
    fsnotify(
        path.inode(),
        FsnotifyEvents::CREATE | FsnotifyEvents::ISDIR,
        Some(name),
    );
}

/// Notifies that a file was created.
pub fn on_create(path: &Path, name: String) {
    if !path.inode().fs().fsnotify_info().is_subscribed() {
        return;
    }
    fsnotify(path.inode(), FsnotifyEvents::CREATE, Some(name));
}

/// Notifies that a file was opened.
pub fn on_open(file: &Arc<dyn FileLike>) {
    // TODO: Check fmode flags (FMODE_NONOTIFY, FMODE_NONOTIFY_PERM).
    if let Some(path) = file.path() {
        // Fast path: check if filesystem has any watchers before doing expensive operations
        if !path.inode().fs().fsnotify_info().is_subscribed() {
            return;
        }
        fsnotify_parent(path, FsnotifyEvents::OPEN, path.effective_name());
    }
}

/// Notifies that a file was closed.
pub fn on_close(file: &Arc<dyn FileLike>) {
    // TODO: Check fmode flags (FMODE_NONOTIFY, FMODE_NONOTIFY_PERM).
    if let Some(path) = file.path() {
        // Fast path: check if filesystem has any watchers before doing expensive operations
        if !path.inode().fs().fsnotify_info().is_subscribed() {
            return;
        }
        let events = match file.access_mode() {
            AccessMode::O_RDONLY => FsnotifyEvents::CLOSE_NOWRITE,
            _ => FsnotifyEvents::CLOSE_WRITE,
        };
        fsnotify_parent(path, events, path.effective_name());
    }
}

/// Notifies that a file's attributes changed.
pub fn on_attr_change(path: &Path) {
    // Fast path: check if filesystem has any watchers before doing expensive operations
    if !path.inode().fs().fsnotify_info().is_subscribed() {
        return;
    }
    fsnotify_parent(path, FsnotifyEvents::ATTRIB, path.effective_name());
}

/// Notifies a path's parent and the path itself about filesystem events.
///
/// If the parent is watching or if subscribers have registered interested events with
/// parent and name information, notifies the parent with child name info.
/// Otherwise, notifies only the child without name information.
/// This function is already called after filesystem checking in the callers.
fn fsnotify_parent(path: &Path, mut events: FsnotifyEvents, name: String) {
    if path.inode().type_() == InodeType::Dir {
        events |= FsnotifyEvents::ISDIR;
    }

    let parent = path.effective_parent();
    if let Some(parent) = parent {
        fsnotify(parent.inode(), events, Some(name));
    }
    fsnotify(path.inode(), events, None);
}

/// Sends a filesystem notification event to all subscribers of an inode.
///
/// This is the main entry point for fsnotify. The VFS layer calls hook-specific
/// functions in `fs/notify/`, which then call this function to broadcast events
/// to all registered subscribers through the inode's publisher.
#[inline]
fn fsnotify(inode: &Arc<dyn Inode>, events: FsnotifyEvents, name: Option<String>) {
    inode.fsnotify_publisher().publish_event(events, name);
}
