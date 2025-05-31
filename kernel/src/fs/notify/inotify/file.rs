// SPDX-License-Identifier: MPL-2.0

use alloc::{string::String, sync::Arc};
use core::sync::atomic::{AtomicU32, Ordering};

use bitflags::bitflags;
use hashbrown::HashMap;
use ostd::{mm::VmWriter, sync::Mutex};

use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        notify::{FsnotifyEvent, FsnotifyFlags, FsnotifyGroup, FsnotifyMark, FsnotifyMarkFlags},
        path::Dentry,
        utils::{Inode, InodeMode, IoctlCmd, Metadata},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    return_errno_with_message,
};

#[derive(Debug)]
pub struct InotifyFile {
    wd_allocator: AtomicU32,
    wd_map: RwLock<HashMap<u32, (Arc<dyn Inode>, Arc<dyn FsnotifyMark>)>>,
    flags: InotifyFlags,
    notifications: RwLock<Vec<Arc<dyn FsnotifyEvent>>>,
    this: Weak<InotifyFile>,
}

impl Drop for InotifyFile {
    fn drop(&mut self) {
        let wd_map = self.wd_map.write();
        for (_, (inode, mark)) in wd_map.iter() {
            inode.remove_fsnotify_mark(mark);
        }
    }
}

impl InotifyFile {
    pub fn new(flags: InotifyFlags) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            wd_allocator: AtomicU32::new(0),
            wd_map: RwLock::new(HashMap::new()),
            flags,
            notifications: RwLock::new(Vec::new()),
            this: weak_self.clone(),
        })
    }

    fn alloc_wd(&self) -> u32 {
        self.wd_allocator.fetch_add(1, Ordering::SeqCst)
    }

    fn find_inode_mark(&self, wd: u32) -> Option<(Arc<dyn Inode>, Arc<dyn FsnotifyMark>)> {
        let wd_map = self.wd_map.read();
        wd_map.get(&wd).cloned()
    }

    fn remove_fsnotify_mark(&self, wd: u32) {
        let mut wd_map = self.wd_map.write();
        wd_map.remove(&wd);
    }

    fn add_fsnotify_mark(&self, wd: u32, inode: Arc<dyn Inode>, mark: Arc<dyn FsnotifyMark>) {
        let mut wd_map = self.wd_map.write();
        wd_map.insert(wd, (inode, mark));
    }

    pub fn update_watch(&self, dentry: &Dentry, mask: u32) -> Result<u32> {
        // try to update and existing watch with the new arg
        let ret = self.inotify_update_existing_watch(dentry, mask);
        match ret {
            Ok(wd) => Ok(wd),
            Err(e) => {
                if e.error() == Errno::ENOENT {
                    // if the watch is not found, create a new watch
                    let wd = self.inotify_new_watch(dentry, mask)?;
                    Ok(wd)
                } else {
                    Err(e)
                }
            }
        }
    }

    // Remove fsnotify mark by watch descriptor
    pub fn inotify_remove_watch(&self, wd: u32) -> Result<()> {
        // find the fsnotify mark from the watch descriptor
        let inode_and_mark = self.find_inode_mark(wd);
        if let Some((inode, mark)) = inode_and_mark {
            // Remove the mark from the inode's mark list
            inode.remove_fsnotify_mark(&mark);
            // Send the IN_IGNORED event representing the mark is removed
            mark.fsnotify_group()
                .send_event(&mark, InotifyMask::IN_IGNORED.bits(), String::new());
            // Remove the mapping between watch descriptor and (inode, mark) pair from the wd_map
            self.remove_fsnotify_mark(wd);
        } else {
            return_errno_with_message!(Errno::EINVAL, "watch not found");
        }
        Ok(())
    }

    // Update existing fsnotify mark
    fn inotify_update_existing_watch(&self, dentry: &Dentry, mask: u32) -> Result<u32> {
        let fsnotify_group = self.this() as Arc<dyn FsnotifyGroup>;
        let mark = dentry.find_fsnotify_mark(&fsnotify_group);
        if let Some(mark) = mark {
            if mask & InotifyMask::IN_MASK_CREATE.bits() != 0 {
                return_errno_with_message!(Errno::EEXIST, "watch already exists");
            }
            mark.update_mark(mask)
        } else {
            return_errno_with_message!(Errno::ENOENT, "watch not found");
        }
    }

    // Create a new fsnotify mark and active it
    fn inotify_new_watch(&self, dentry: &Dentry, arg: u32) -> Result<u32> {
        let mask = inotify_arg_to_mask(arg);
        let flags = inotify_arg_to_flags(arg);
        let inotify_mark = InotifyMark::new(self.this(), mask, flags);
        // Add the mark to the inode's mark list
        let fsnotify_mark = inotify_mark.clone() as Arc<dyn FsnotifyMark>;
        dentry.inode().add_fsnotify_mark(fsnotify_mark.clone(), 0);
        // Store the mapping between watch descriptor and (inode, mark) pair
        let wd = inotify_mark.wd();
        self.add_fsnotify_mark(wd, dentry.inode().clone(), fsnotify_mark.clone());
        Ok(wd)
    }

    fn this(&self) -> Arc<InotifyFile> {
        self.this.upgrade().unwrap()
    }
}

impl FsnotifyGroup for InotifyFile {
    fn send_event(&self, mark: &Arc<dyn FsnotifyMark>, mask: u32, name: String) {
        let wd = mark.downcast_ref::<InotifyMark>().unwrap().wd();
        let mark_mask = mark
            .downcast_ref::<InotifyMark>()
            .unwrap()
            .inner
            .lock()
            .mask;
        if mark_mask & mask == 0 && mask != InotifyMask::IN_IGNORED.bits() {
            return;
        }
        let event = Arc::new(InotifyEvent::new(
            mask,
            wd,
            0,
            (name.len() + 1) as u32,
            name,
        ));
        // TODO: inotify merge event
        self.notifications.write().push(event);
    }

    fn pop_event(&self) -> Option<Arc<dyn FsnotifyEvent>> {
        let mut notifications = self.notifications.write();
        if notifications.is_empty() {
            None
        } else {
            Some(notifications.remove(0))
        }
    }

    fn get_all_event_size(&self) -> usize {
        self.notifications
            .read()
            .iter()
            .map(|event| event.get_size())
            .sum()
    }

    // Free fsnotify mark of inotify file
    fn free_mark(&self, mark: &Arc<dyn FsnotifyMark>) {
        // Send the IN_IGNORED event representing the mark is removed
        mark.fsnotify_group()
            .send_event(mark, InotifyMask::IN_IGNORED.bits(), String::new());
        let wd = mark.downcast_ref::<InotifyMark>().unwrap().wd();
        self.remove_fsnotify_mark(wd);
    }
}

impl Pollable for InotifyFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        warn!("Inotify file doesn't support poll now, return IoEvents::empty for now.");
        IoEvents::empty()
    }
}

impl FileLike for InotifyFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if self.flags.contains(InotifyFlags::IN_NONBLOCK) && self.get_all_event_size() == 0 {
            return_errno_with_message!(Errno::EAGAIN, "non-blocking read");
        }

        let mut size = 0;
        while let Some(event) = self.pop_event() {
            size += event.copy_to_user(writer)?;
        }
        Ok(size)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::FIONREAD => {
                let size = self.get_all_event_size();
                current_userspace!().write_val(arg, &size)?;
                Ok(0)
            }
            _ => return_errno_with_message!(Errno::EINVAL, "ioctl is not supported"),
        }
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "anonymous inode fs" and link `InotifyFile` to it.
        Metadata::new_file(
            0,
            InodeMode::from_bits_truncate(0o600),
            aster_block::BLOCK_SIZE,
        )
    }
}

fn inotify_arg_to_mask(arg: u32) -> u32 {
    let mut mask = FsnotifyFlags::FS_UNMOUNT.bits();
    mask |= arg & InotifyMask::IN_ALL_EVENTS.bits();
    mask
}

fn inotify_arg_to_flags(arg: u32) -> u32 {
    let mut flag = 0;
    if arg & InotifyMask::IN_EXCL_UNLINK.bits() != 0 {
        flag |= FsnotifyMarkFlags::FSNOTIFY_MARK_FLAG_EXCL_UNLINK.bits();
    }

    if arg & InotifyMask::IN_ONESHOT.bits() != 0 {
        flag |= FsnotifyMarkFlags::FSNOTIFY_MARK_FLAG_IN_ONESHOT.bits();
    }

    flag
}

bitflags! {
    pub struct InotifyMask: u32 {
        // Core events that user-space can watch for
        const IN_ACCESS        = 1 << 0;  // File was accessed
        const IN_MODIFY        = 1 << 1;  // File was modified
        const IN_ATTRIB        = 1 << 2;  // Metadata changed
        const IN_CLOSE_WRITE   = 1 << 3;  // Writable file was closed
        const IN_CLOSE_NOWRITE = 1 << 4;  // Unwritable file closed
        const IN_OPEN          = 1 << 5;  // File was opened
        const IN_MOVED_FROM    = 1 << 6;  // File was moved from X
        const IN_MOVED_TO      = 1 << 7;  // File was moved to Y
        const IN_CREATE        = 1 << 8;  // Subfile was created
        const IN_DELETE        = 1 << 9;  // Subfile was deleted
        const IN_DELETE_SELF   = 1 << 10; // Self was deleted
        const IN_MOVE_SELF     = 1 << 11; // Self was moved

        // Additional events sent as needed
        const IN_UNMOUNT       = 1 << 13; // Backing fs was unmounted
        const IN_Q_OVERFLOW    = 1 << 14; // Event queue overflowed
        const IN_IGNORED       = 1 << 15; // File was ignored

        // Helper events
        const IN_CLOSE         = Self::IN_CLOSE_WRITE.bits() | Self::IN_CLOSE_NOWRITE.bits(); // Close events
        const IN_MOVE          = Self::IN_MOVED_FROM.bits() | Self::IN_MOVED_TO.bits();       // Move events

        // Special flags
        const IN_ONLYDIR       = 1 << 24; // Only watch directories
        const IN_DONT_FOLLOW   = 1 << 25; // Don't follow symlinks
        const IN_EXCL_UNLINK   = 1 << 26; // Exclude events on unlinked objects
        const IN_MASK_CREATE   = 1 << 28; // Only create watches
        const IN_MASK_ADD      = 1 << 29; // Add to existing watch mask
        const IN_ISDIR         = 1 << 30; // Event occurred on a directory
        const IN_ONESHOT       = 1 << 31; // Send event once
        const IN_ALL_EVENTS    = Self::IN_ACCESS.bits() | Self::IN_MODIFY.bits() | Self::IN_ATTRIB.bits() |
                                 Self::IN_CLOSE_WRITE.bits() | Self::IN_CLOSE_NOWRITE.bits() | Self::IN_OPEN.bits() |
                                 Self::IN_MOVED_FROM.bits() | Self::IN_MOVED_TO.bits() | Self::IN_DELETE.bits() |
                                 Self::IN_CREATE.bits() | Self::IN_DELETE_SELF.bits() | Self::IN_MOVE_SELF.bits();
    }
}

#[derive(Debug)]
struct InotifyMark {
    inner: Mutex<InotifyMarkInner>,
    wd: u32,
    inotify_file: Arc<InotifyFile>,
}

#[derive(Debug)]
struct InotifyMarkInner {
    mask: u32,
    flags: u32,
}

impl InotifyMark {
    pub fn new(inotify_file: Arc<InotifyFile>, mask: u32, flags: u32) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(InotifyMarkInner { mask, flags }),
            wd: inotify_file.alloc_wd(),
            inotify_file,
        })
    }

    pub fn wd(&self) -> u32 {
        self.wd
    }
}

impl FsnotifyMark for InotifyMark {
    fn fsnotify_group(&self) -> Arc<dyn FsnotifyGroup> {
        self.inotify_file.clone() as Arc<dyn FsnotifyGroup>
    }

    fn update_mark(&self, arg: u32) -> Result<u32> {
        let mut mark = self.inner.lock();
        let mask = inotify_arg_to_mask(arg);
        if mask & InotifyMask::IN_MASK_CREATE.bits() != 0 {
            return_errno_with_message!(Errno::EEXIST, "watch already exists");
        }

        if mask & InotifyMask::IN_MASK_ADD.bits() == 0 {
            mark.mask = 0;
            mark.flags = mark.flags
                & !(FsnotifyMarkFlags::FSNOTIFY_MARK_FLAG_ATTACHED.bits()
                    | FsnotifyMarkFlags::FSNOTIFY_MARK_FLAG_IN_ONESHOT.bits());
        }

        mark.mask = mark.mask | inotify_arg_to_mask(mask);
        mark.flags = mark.flags | inotify_arg_to_flags(mask);

        if mark.mask != mask {
            // TODO: implement update fsnotify mask
        }

        Ok(self.wd())
    }
}

#[derive(Debug)]
struct InotifyEvent {
    wd: u32,
    mask: u32,
    cookie: u32,
    name_len: u32,
    name: String,
}

impl InotifyEvent {
    pub fn new(mask: u32, wd: u32, cookie: u32, name_len: u32, name: String) -> Self {
        Self {
            mask,
            wd,
            cookie,
            name_len,
            name,
        }
    }
}

impl FsnotifyEvent for InotifyEvent {
    fn copy_to_user(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut total_size = 0;

        // Write the event header
        writer.write_val(&self.wd)?;
        writer.write_val(&self.mask)?;
        writer.write_val(&self.cookie)?;
        writer.write_val(&self.name_len)?;
        total_size += core::mem::size_of::<u32>() * 4;
        if self.name.len() > 0 {
            let bytes = self.name.as_bytes();
            for byte in bytes {
                writer.write_val(byte)?;
            }
        }
        writer.write_val(&b'\0')?;
        total_size += self.name.len() + 1;
        Ok(total_size)
    }

    fn get_size(&self) -> usize {
        core::mem::size_of::<u32>() * 4 + self.name.len() + 1
    }
}

bitflags! {
    pub struct InotifyFlags: u32 {
        const IN_NONBLOCK = 1 << 11; // Non-blocking
        const IN_CLOEXEC = 1 << 19; // Close on exec
    }
}
