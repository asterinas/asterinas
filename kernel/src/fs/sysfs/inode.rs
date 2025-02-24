// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use ostd::sync::RwMutex;

use crate::{
    events::{Events, EventsFilter, Observer, Subject},
    fs::{
        kernfs::{DataProvider, KernfsNode, PseudoFileSystem, PseudoNode},
        utils::{Inode, InodeMode, InodeType},
    },
    prelude::*,
};

/// Action of KObject event.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    Add,
    Remove,
    Offline,
    Online,
    Change,
    Move,
}

/// UEvent represents a KObject event, corresponding to a UEvent in Linux.
///
/// It is used to notify observers of KObject events.
/// FIXME: Currently, the UEvent is a placeholder and does not contain any useful information.
#[derive(Debug, Clone, Copy)]
pub struct UEvent {
    action: Action,
}

impl UEvent {
    pub fn new(action: Action) -> Self {
        UEvent { action }
    }
}

impl Events for UEvent {}

#[derive(Debug, Clone, Copy)]
pub struct UEventFilter;

impl EventsFilter<UEvent> for UEventFilter {
    fn filter(&self, event: &UEvent) -> bool {
        false
    }
}

/// The core struct of the sysfs, corresponding to the KObject and KSet in Linux.
///
/// Represents a kernel object in the kernel filesystem, combining features of
/// Linux sysfs kobject and kset. This struct manages the lifecycle of a kernel object
/// and its relationships within the kernfs hierarchy.
pub struct KObject {
    node: Arc<KernfsNode>,                  // The associated KernfsNode
    subject: Subject<UEvent, UEventFilter>, // Subject for event notifications
    parent: Option<Weak<KObject>>,          // Parent of
    this: Weak<KObject>,                    // Weak reference to self
}

impl KObject {
    pub fn get_node(&self) -> Arc<KernfsNode> {
        self.node.clone()
    }

    pub fn notify_observers(&self, event: UEvent) {
        debug!("KObject: {}, uevent: {:?}", self.name(), event);
        self.subject.notify_observers(&event);
    }

    pub fn new_root(
        name: &str,
        fs: Weak<dyn PseudoFileSystem>,
        root_ino: u64,
        blk_size: usize,
    ) -> Arc<Self> {
        let node = KernfsNode::new_root(name, fs, root_ino, blk_size);
        Arc::new_cyclic(|weak_kobject| Self {
            node,
            subject: Subject::new(),
            parent: None,
            this: weak_kobject.clone(),
        })
    }

    /// Creates a new dir KObject.
    pub fn new_dir(name: &str, mode: u16, parent: Option<Weak<KObject>>) -> Result<Arc<Self>> {
        let mode = InodeMode::from_bits_truncate(mode);
        let parent_node: Arc<dyn PseudoNode> = parent.as_ref().and_then(|p| p.upgrade()).unwrap();
        let node = KernfsNode::new_dir(
            name,
            Some(mode),
            Arc::downgrade(&parent_node),
        )?;

        let this = Arc::new_cyclic(|weak_kobject| Self {
            node,
            subject: Subject::new(),
            parent: parent.clone(),
            this: weak_kobject.clone(),
        });
        parent_node.insert(name.to_string(), this.clone())?;
        if let Some(parent) = parent {
            parent
                .upgrade()
                .unwrap()
                .notify_observers(UEvent::new(Action::Add));
        }
        Ok(this)
    }

    /// Creates a new symlink KObject.
    pub fn new_link(
        name: &str,
        parent: Option<Weak<KObject>>,
        target: Arc<dyn PseudoNode>,
    ) -> Result<Arc<Self>> {
        let parent_node: Arc<dyn PseudoNode> = parent.as_ref().and_then(|p| p.upgrade()).unwrap();
        let node = KernfsNode::new_symlink(
            name,
            target,
            Arc::downgrade(&parent_node),
        )?;

        let this = Arc::new_cyclic(|weak_kobject| Self {
            node,
            subject: Subject::new(),
            parent: parent.clone(),
            this: weak_kobject.clone(),
        });
        parent_node.insert(name.to_string(), this.clone())?;
        if let Some(parent) = parent {
            parent
                .upgrade()
                .unwrap()
                .notify_observers(UEvent::new(Action::Add));
        }

        Ok(this)
    }

    /// Creates a new Attribute KObject.
    pub fn new_attr(name: &str, mode: u16, parent: Option<Weak<KObject>>) -> Result<Arc<Self>> {
        let mode = InodeMode::from_bits_truncate(mode);
        let parent_node: Arc<dyn PseudoNode> = parent.as_ref().and_then(|p| p.upgrade()).unwrap();
        let node = KernfsNode::new_attr(
            name,
            Some(mode),
            Arc::downgrade(&parent_node),
        )?;

        let this = Arc::new_cyclic(|weak_kobject| Self {
            node,
            subject: Subject::new(),
            parent: parent.clone(),
            this: weak_kobject.clone(),
        });
        parent_node.insert(name.to_string(), this.clone())?;
        if let Some(parent) = parent {
            parent
                .upgrade()
                .unwrap()
                .notify_observers(UEvent::new(Action::Add));
        }

        Ok(this)
    }
}

impl PseudoNode for KObject {
    fn name(&self) -> String {
        self.node.name()
    }

    fn parent(&self) -> Option<Arc<dyn PseudoNode>> {
        self.parent
            .as_ref()
            .and_then(|p| p.upgrade())
            .map(|arc| arc as Arc<dyn PseudoNode>)
    }

    fn pseudo_fs(&self) -> Arc<dyn PseudoFileSystem> {
        self.node.pseudo_fs()
    }

    fn generate_ino(&self) -> u64 {
        self.node.generate_ino()
    }

    fn set_data(&self, data: Box<dyn DataProvider>) -> Result<()> {
        self.node.set_data(data)
    }

    fn remove(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let child = self.node.remove(name)?;
        self.notify_observers(UEvent::new(Action::Remove));
        Ok(child)
    }

    fn insert(&self, name: String, node: Arc<dyn Inode>) -> Result<()> {
        self.node.insert(name, node)?;
        self.notify_observers(UEvent::new(Action::Add));
        Ok(())
    }
}

impl Clone for KObject {
    fn clone(&self) -> Self {
        Self {
            node: self.node.clone(),
            subject: Subject::new(),
            parent: self.parent.clone(),
            this: self.this.clone(),
        }
    }
}

impl KObject {
    pub fn this(&self) -> Arc<KObject> {
        self.this.upgrade().unwrap()
    }

    pub fn this_weak(&self) -> Weak<KObject> {
        self.this.clone()
    }

    pub fn register_observer(&self, observer: Weak<dyn Observer<UEvent>>, mask: UEventFilter) {
        self.subject.register_observer(observer, mask);
    }

    pub fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<UEvent>>,
    ) -> Option<Weak<dyn Observer<UEvent>>> {
        self.subject.unregister_observer(observer)
    }
}

impl Drop for KObject {
    fn drop(&mut self) {
        if let Some(parent) = &self.parent {
            if let Some(parent) = parent.upgrade() {
                parent.remove(&self.name());
            }
        }
    }
}

impl Inode for KObject {
    fn size(&self) -> usize {
        self.node.size()
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        self.node.resize(new_size)
    }

    fn metadata(&self) -> crate::fs::utils::Metadata {
        self.node.metadata()
    }

    fn ino(&self) -> u64 {
        self.node.ino()
    }

    fn type_(&self) -> crate::fs::utils::InodeType {
        self.node.type_()
    }

    fn mode(&self) -> Result<InodeMode> {
        self.node.mode()
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.node.set_mode(mode)
    }

    fn owner(&self) -> Result<crate::process::Uid> {
        self.node.owner()
    }

    fn set_owner(&self, uid: crate::process::Uid) -> Result<()> {
        self.node.set_owner(uid)
    }

    fn group(&self) -> Result<crate::process::Gid> {
        self.node.group()
    }

    fn set_group(&self, gid: crate::process::Gid) -> Result<()> {
        self.node.set_group(gid)
    }

    fn atime(&self) -> core::time::Duration {
        self.node.atime()
    }

    fn set_atime(&self, time: core::time::Duration) {
        self.node.set_atime(time)
    }

    fn mtime(&self) -> core::time::Duration {
        self.node.mtime()
    }

    fn set_mtime(&self, time: core::time::Duration) {
        self.node.set_mtime(time)
    }

    fn ctime(&self) -> core::time::Duration {
        self.node.ctime()
    }

    fn set_ctime(&self, time: core::time::Duration) {
        self.node.set_ctime(time)
    }

    fn fs(&self) -> Arc<dyn crate::fs::utils::FileSystem> {
        self.node.fs()
    }

    fn page_cache(&self) -> Option<crate::vm::vmo::Vmo<aster_rights::Full>> {
        self.node.page_cache()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.node.read_at(offset, writer)
    }

    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.node.read_direct_at(offset, writer)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let size = self.node.write_at(offset, reader)?;
        self.notify_observers(UEvent::new(Action::Change));
        Ok(size)
    }

    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let size = self.node.write_direct_at(offset, reader)?;
        self.notify_observers(UEvent::new(Action::Change));
        Ok(size)
    }

    fn create(
        &self,
        name: &str,
        type_: crate::fs::utils::InodeType,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.lookup(name).is_ok() {
            return_errno!(Errno::EEXIST);
        }
        let new_node = match type_ {
            InodeType::Dir => KObject::new_dir(name, mode.bits(), Some(self.this_weak()))?,
            InodeType::File => KObject::new_attr(name, mode.bits(), Some(self.this_weak()))?,
            _ => return_errno!(Errno::EINVAL),
        };
        Ok(new_node)
    }

    fn mknod(
        &self,
        name: &str,
        mode: InodeMode,
        type_: crate::fs::utils::MknodType,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn as_device(&self) -> Option<Arc<dyn crate::fs::device::Device>> {
        self.node.as_device()
    }

    fn readdir_at(
        &self,
        offset: usize,
        visitor: &mut dyn crate::fs::utils::DirentVisitor,
    ) -> Result<usize> {
        self.node.readdir_at(offset, visitor)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        if old.type_() != InodeType::File && old.type_() != InodeType::Dir {
            return_errno!(Errno::EPERM);
        }
        if name == "." || name == ".." {
            return_errno!(Errno::EPERM);
        }
        if self.lookup(name).is_ok() {
            return_errno!(Errno::EEXIST);
        }
        let target = old
            .downcast_ref::<KObject>()
            .ok_or(Error::new(Errno::EXDEV))?
            .this();
        let new_node = KObject::new_link(name, Some(self.this_weak()), target)?;
        Ok(())
    }

    fn unlink(&self, name: &str) -> Result<()> {
        self.rmdir(name)
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        let child = self.remove(name)?;
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        self.node.lookup(name)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        self.node.rename(old_name, target, new_name)
    }

    fn read_link(&self) -> Result<String> {
        self.node.read_link()
    }

    fn write_link(&self, target: &str) -> Result<()> {
        self.node.write_link(target)
    }

    fn ioctl(&self, cmd: crate::fs::utils::IoctlCmd, arg: usize) -> Result<i32> {
        self.node.ioctl(cmd, arg)
    }

    fn sync_all(&self) -> Result<()> {
        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        Ok(())
    }

    fn fallocate(
        &self,
        mode: crate::fs::utils::FallocMode,
        offset: usize,
        len: usize,
    ) -> Result<()> {
        return_errno!(Errno::EOPNOTSUPP);
    }

    fn poll(
        &self,
        mask: crate::events::IoEvents,
        _poller: Option<&mut crate::process::signal::PollHandle>,
    ) -> crate::events::IoEvents {
        let events = crate::events::IoEvents::IN | crate::events::IoEvents::OUT;
        events & mask
    }

    fn is_dentry_cacheable(&self) -> bool {
        true
    }

    fn is_seekable(&self) -> bool {
        true
    }

    fn extension(&self) -> Option<&crate::fs::utils::Extension> {
        None
    }
}
