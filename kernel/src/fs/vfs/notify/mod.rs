// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::{
    any::Any,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use bitflags::bitflags;
use ostd::sync::RwLock;

use crate::{
    fs::{file_handle::FileLike, path::Path, utils::AccessMode},
    prelude::*,
};

pub mod inotify;

use super::utils::{Inode, InodeExt, InodeType};

/// Publishes filesystem events to subscribers.
///
/// Each inode has an associated `FsEventPublisher` that maintains a list of
/// subscribers interested in filesystem events. When an event occurs, the publisher
/// notifies all subscribers whose interesting events match the event.
pub struct FsEventPublisher {
    /// List of FS event subscribers.
    subscribers: RwLock<Vec<Arc<dyn FsEventSubscriber>>>,
    /// All interesting FS event types (aggregated from all subscribers).
    all_interesting_events: AtomicFsEvents,
    /// Whether this publisher still accepts new subscribers.
    accepts_new_subscribers: AtomicBool,
}

impl Debug for FsEventPublisher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FsEventPublisher")
            .field("num_subscribers", &self.subscribers.read().len())
            .finish_non_exhaustive()
    }
}

impl Default for FsEventPublisher {
    fn default() -> Self {
        Self::new()
    }
}

impl FsEventPublisher {
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
            all_interesting_events: AtomicFsEvents::new(FsEvents::empty()),
            accepts_new_subscribers: AtomicBool::new(true),
        }
    }

    /// Adds a subscriber to this publisher.
    pub fn add_subscriber(&self, subscriber: Arc<dyn FsEventSubscriber>) -> bool {
        let mut subscribers = self.subscribers.write();

        // This check must be done after locking `self.subscribers.write()` to avoid race
        // conditions.
        if !self.accepts_new_subscribers.load(Ordering::Relaxed) {
            return false;
        }

        let current = self.all_interesting_events.load(Ordering::Relaxed);
        self.all_interesting_events
            .store(current | subscriber.interesting_events(), Ordering::Relaxed);

        subscribers.push(subscriber);

        true
    }

    /// Removes a subscriber from this publisher.
    pub fn remove_subscriber(&self, subscriber: &Arc<dyn FsEventSubscriber>) -> bool {
        let mut subscribers = self.subscribers.write();

        let orig_len = subscribers.len();
        self.retain_and_recalc_events(&mut subscribers, |m| !Arc::ptr_eq(m, subscriber));

        let removed = subscribers.len() != orig_len;
        if removed {
            subscriber.deliver_event(FsEvents::IN_IGNORED, None);
        }

        removed
    }

    /// Removes all subscribers from this publisher.
    pub fn remove_all_subscribers(&self) -> usize {
        let mut subscribers = self.subscribers.write();

        for subscriber in subscribers.iter() {
            subscriber.deliver_event(FsEvents::IN_IGNORED, None);
        }

        let num_subscribers = subscribers.len();
        subscribers.clear();

        self.all_interesting_events
            .store(FsEvents::empty(), Ordering::Relaxed);

        num_subscribers
    }

    /// Forbids new subscribers from attaching to this publisher and removes all existing
    /// subscribers.
    pub fn disable_new_and_remove_subscribers(&self) -> usize {
        // Do this before calling `remove_all_subscribers` so that the `self.subscribers.write()`
        // lock will synchronize this variable.
        self.accepts_new_subscribers.store(false, Ordering::Relaxed);

        self.remove_all_subscribers()
    }

    /// Broadcasts an event to all the subscribers of this publisher.
    pub fn publish_event(&self, events: FsEvents, name: Option<String>) {
        let interesting = self.all_interesting_events.load(Ordering::Relaxed);
        if !interesting.intersects(events) {
            return;
        }

        let subscribers = self.subscribers.read();
        let mut has_oneshot = false;
        for subscriber in subscribers.iter() {
            has_oneshot |= subscriber.deliver_event(events, name.clone());
        }
        drop(subscribers);

        if has_oneshot {
            let mut subscribers = self.subscribers.write();
            // The `deliver_event()` method should already deliver the `FsEvents::IN_IGNORED`
            // events for one-shot subscribers. Here, we simply remove them.
            self.retain_and_recalc_events(&mut subscribers, |m| !m.is_oneshot_and_dead());
        }
    }

    /// Updates the aggregated events when a subscriber's interesting events change.
    pub fn update_subscriber_events(&self) {
        // Take a write lock to avoid race conditions that may change `all_interesting_events` to
        // an outdated value.
        let mut subscribers = self.subscribers.write();
        self.retain_and_recalc_events(&mut subscribers, |_| true);
    }

    /// Retains only the subscribers specified by the predicate and recalculates the aggregated
    /// interesting events.
    fn retain_and_recalc_events<F>(
        &self,
        subscribers: &mut Vec<Arc<dyn FsEventSubscriber>>,
        mut pred: F,
    ) where
        F: FnMut(&Arc<dyn FsEventSubscriber>) -> bool,
    {
        let mut new_events = FsEvents::empty();
        subscribers.retain(|subscriber| {
            if pred(subscriber) {
                new_events |= subscriber.interesting_events();
                true
            } else {
                false
            }
        });
        self.all_interesting_events
            .store(new_events, Ordering::Relaxed);
    }

    /// Finds a subscriber and applies an action if found.
    ///
    /// The matcher should return `Some(T)` if the subscriber matches and processing
    /// should stop, or `None` to continue searching.
    #[expect(dead_code)]
    pub fn find_subscriber_and_process<F, T>(&self, mut matcher: F) -> Option<T>
    where
        F: FnMut(&Arc<dyn FsEventSubscriber>) -> Option<T>,
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

/// Represents a subscriber to filesystem events on an `FsEventPublisher`.
///
/// A subscriber receives notifications from a publisher when filesystem events occur
/// that match the subscriber's interesting events. The subscriber specifies which events
/// it is interested in using `FsEvents`, which define the types of events (e.g.,
/// read, write, modify, delete) the subscriber wants to be notified about. When an event
/// occurs, the publisher (attached to an inode) broadcasts it to all subscribers whose
/// interesting events match the event type.
pub trait FsEventSubscriber: Any + Send + Sync {
    /// Delivers a filesystem event notification to the subscriber.
    ///
    /// Returns whether the subscriber is a one-shot subscriber and the event has been
    /// delivered. If there are no one-shot subscribers, simply return `false` here.
    /// Otherwise, [`Self::is_oneshot_and_dead`] should be implemented correspondingly.
    ///
    /// Invariant: This method must not sleep or perform blocking operations. The publisher
    /// may hold a spin lock when calling this method.
    fn deliver_event(&self, events: FsEvents, name: Option<String>) -> bool;

    /// Returns the events that this subscriber is interested in.
    fn interesting_events(&self) -> FsEvents;

    /// Returns whether the subscriber is a one-shot subscriber and an event has been
    /// delivered.
    ///
    /// This method should return `true` if and only if a previous call to
    /// [`Self::deliver_event`] has already returned `true`.
    fn is_oneshot_and_dead(&self) -> bool {
        false
    }
}

bitflags! {
    /// Represents filesystem events that have occurred.
    ///
    /// These events are used to notify subscribers about specific filesystem actions.
    /// Subscribers specify which events they are interested in to filter and receive
    /// only the events they care about.
    pub struct FsEvents: u32 {
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

impl From<u32> for FsEvents {
    fn from(value: u32) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<FsEvents> for u32 {
    fn from(value: FsEvents) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(FsEvents, {
    #[derive(Debug)]
    pub(super) struct AtomicFsEvents(AtomicU32);
});

/// Notifies that a file was accessed.
pub fn on_access(file: &Arc<dyn FileLike>) {
    // TODO: Check fmode flags (FMODE_NONOTIFY, FMODE_NONOTIFY_PERM).
    let path = file.path();

    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::ACCESS);
}

/// Notifies that a file was modified.
pub fn on_modify(file: &Arc<dyn FileLike>) {
    // TODO: Check fmode flags (FMODE_NONOTIFY, FMODE_NONOTIFY_PERM).
    let path = file.path();

    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::MODIFY);
}

/// Notifies that a path's content was changed.
pub fn on_change(path: &Path) {
    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::MODIFY);
}

/// Notifies that a file was deleted from a directory.
pub fn on_delete(
    dir_inode: &Arc<dyn Inode>,
    inode: &Arc<dyn Inode>,
    name: impl FnOnce() -> String,
) {
    if !dir_inode
        .fs()
        .fs_event_subscriber_stats()
        .has_any_subscribers()
    {
        return;
    }
    if inode.type_() == InodeType::Dir {
        notify_inode_with_name(dir_inode, FsEvents::DELETE | FsEvents::ISDIR, name)
    } else {
        notify_inode_with_name(dir_inode, FsEvents::DELETE, name)
    }
}

/// Notifies that an inode's link count changed.
pub fn on_link_count(inode: &Arc<dyn Inode>) {
    if !inode.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_inode(inode, FsEvents::ATTRIB);
}

/// Notifies that an inode was removed (link count reached 0).
pub fn on_inode_removed(inode: &Arc<dyn Inode>) {
    if !inode.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_inode(inode, FsEvents::DELETE_SELF);
}

/// Notifies that a file was linked to a directory.
pub fn on_link(dir_inode: &Arc<dyn Inode>, inode: &Arc<dyn Inode>, name: impl FnOnce() -> String) {
    if !dir_inode
        .fs()
        .fs_event_subscriber_stats()
        .has_any_subscribers()
    {
        return;
    }
    notify_inode(inode, FsEvents::ATTRIB);
    notify_inode_with_name(dir_inode, FsEvents::CREATE, name);
}

/// Notifies that a directory was created.
pub fn on_mkdir(dir_path: &Path, name: impl FnOnce() -> String) {
    if !dir_path
        .fs()
        .fs_event_subscriber_stats()
        .has_any_subscribers()
    {
        return;
    }
    notify_inode_with_name(dir_path.inode(), FsEvents::CREATE | FsEvents::ISDIR, name);
}

/// Notifies that a file was created.
pub fn on_create(file_path: &Path, name: impl FnOnce() -> String) {
    if !file_path
        .fs()
        .fs_event_subscriber_stats()
        .has_any_subscribers()
    {
        return;
    }
    notify_inode_with_name(file_path.inode(), FsEvents::CREATE, name);
}

/// Notifies that a file was opened.
pub fn on_open(file: &Arc<dyn FileLike>) {
    // TODO: Check fmode flags (FMODE_NONOTIFY, FMODE_NONOTIFY_PERM).
    let path = file.path();

    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::OPEN);
}

/// Notifies that a file was closed.
pub fn on_close(file: &Arc<dyn FileLike>) {
    // TODO: Check fmode flags (FMODE_NONOTIFY, FMODE_NONOTIFY_PERM).
    let path = file.path();

    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    let events = match file.access_mode() {
        AccessMode::O_RDONLY => FsEvents::CLOSE_NOWRITE,
        _ => FsEvents::CLOSE_WRITE,
    };
    notify_parent(path, events);
}

/// Notifies that a file's attributes changed.
pub fn on_attr_change(path: &Path) {
    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::ATTRIB);
}

/// Notifies a path's parent and the path itself about filesystem events.
///
/// If the parent is watching or if subscribers have registered interesting events with
/// parent and name information, notifies the parent with child name info.
/// Otherwise, notifies only the child without name information.
/// This function is already called after filesystem checking in the callers.
///
/// The child's real name (from `path.name()`) is used to notify the parent, since
/// FS events do not cross mount boundaries.
fn notify_parent(path: &Path, mut events: FsEvents) {
    if path.inode().type_() == InodeType::Dir {
        events |= FsEvents::ISDIR;
    }

    let parent = path.parent_within_mount();
    if let Some(parent) = parent {
        notify_inode_with_name(parent.inode(), events, || path.name());
    }
    notify_inode(path.inode(), events);
}

/// Sends a filesystem notification event to all subscribers of an inode.
///
/// This is the main entry point for FS event notification. The VFS layer calls hook-specific
/// functions in `fs/notify/`, which then call this function to broadcast events
/// to all registered subscribers through the inode's publisher.
fn notify_inode(inode: &Arc<dyn Inode>, events: FsEvents) {
    if let Some(publisher) = inode.fs_event_publisher() {
        publisher.publish_event(events, None);
    }
}

/// Sends a filesystem notification event with a name to all subscribers of an inode.
///
/// Similar to `notify_inode`, but includes a name parameter for events that require
/// child name information (e.g., CREATE, DELETE).
fn notify_inode_with_name(inode: &Arc<dyn Inode>, events: FsEvents, name: impl FnOnce() -> String) {
    if let Some(publisher) = inode.fs_event_publisher() {
        publisher.publish_event(events, Some(name()));
    }
}
