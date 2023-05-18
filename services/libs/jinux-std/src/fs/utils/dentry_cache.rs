use crate::fs::device::Device;
use crate::prelude::*;

use alloc::string::String;
use core::time::Duration;

use super::{InodeMode, InodeType, Metadata, Vnode, NAME_MAX};

lazy_static! {
    static ref DCACHE: Mutex<BTreeMap<DentryKey, Arc<Dentry>>> = Mutex::new(BTreeMap::new());
}

/// The dentry cache to accelerate path lookup
pub struct Dentry {
    vnode: Vnode,
    name_and_parent: RwLock<(String, Option<Arc<Dentry>>)>,
    this: Weak<Dentry>,
    children: Mutex<Children>,
}

impl Dentry {
    /// Create a new dentry cache with root inode
    pub fn new_root(root_vnode: Vnode) -> Arc<Self> {
        let root = Self::new("/", None, root_vnode);
        DCACHE.lock().insert(root.key(), root.clone());
        root
    }

    /// Internal constructor
    fn new(name: &str, parent: Option<Arc<Dentry>>, vnode: Vnode) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            vnode,
            name_and_parent: RwLock::new((String::from(name), parent)),
            this: weak_self.clone(),
            children: Mutex::new(Children::new()),
        })
    }

    /// Get the name of Dentry.
    pub fn name(&self) -> String {
        self.name_and_parent.read().0.clone()
    }

    /// Get the parent dentry.
    ///
    /// Returns None if it is root dentry.
    pub fn parent(&self) -> Option<Arc<Dentry>> {
        self.name_and_parent.read().1.clone()
    }

    fn set_name_and_parent(&self, name: &str, parent: Option<Arc<Dentry>>) {
        let mut name_and_parent = self.name_and_parent.write();
        name_and_parent.0 = String::from(name);
        name_and_parent.1 = parent;
    }

    fn this(&self) -> Arc<Dentry> {
        self.this.upgrade().unwrap()
    }

    fn key(&self) -> DentryKey {
        let parent = self.parent().unwrap_or(self.this());
        DentryKey::new(&self.name_and_parent.read().0, &parent)
    }

    pub fn vnode(&self) -> &Vnode {
        &self.vnode
    }

    /// Create a dentry by making inode.
    pub fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<Self>> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        if children.find_dentry(name).is_some() {
            return_errno!(Errno::EEXIST);
        }

        let child = {
            let vnode = self.vnode.create(name, type_, mode)?;
            let dentry = Dentry::new(name, Some(self.this()), vnode);
            children.insert_dentry(&dentry);
            dentry
        };
        Ok(child)
    }

    /// Create a dentry by making a device inode.
    pub fn mknod(&self, name: &str, mode: InodeMode, device: Arc<dyn Device>) -> Result<Arc<Self>> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        if children.find_dentry(name).is_some() {
            return_errno!(Errno::EEXIST);
        }

        let child = {
            let vnode = self.vnode.mknod(name, mode, device)?;
            let dentry = Dentry::new(name, Some(self.this()), vnode);
            children.insert_dentry(&dentry);
            dentry
        };
        Ok(child)
    }

    /// Lookup a dentry.
    pub fn lookup(&self, name: &str) -> Result<Arc<Self>> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let dentry = match name {
            "." => self.this(),
            ".." => self.parent().unwrap_or(self.this()),
            name => {
                let mut children = self.children.lock();
                match children.find_dentry(name) {
                    Some(dentry) => dentry.clone(),
                    None => {
                        let vnode = self.vnode.lookup(name)?;
                        let dentry = Dentry::new(name, Some(self.this()), vnode);
                        children.insert_dentry(&dentry);
                        dentry
                    }
                }
            }
        };
        Ok(dentry)
    }

    /// Link a new name for the dentry by linking inode.
    pub fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        if children.find_dentry(name).is_some() {
            return_errno!(Errno::EEXIST);
        }
        let old_vnode = old.vnode();
        self.vnode.link(old_vnode, name)?;
        let dentry = Dentry::new(name, Some(self.this()), old_vnode.clone());
        children.insert_dentry(&dentry);
        Ok(())
    }

    /// Delete a dentry by unlinking inode.
    pub fn unlink(&self, name: &str) -> Result<()> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        self.vnode.unlink(name)?;
        children.delete_dentry(name);
        Ok(())
    }

    /// Delete a directory dentry by rmdiring inode.
    pub fn rmdir(&self, name: &str) -> Result<()> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut children = self.children.lock();
        self.vnode.rmdir(name)?;
        children.delete_dentry(name);
        Ok(())
    }

    /// Read symbolic link.
    pub fn read_link(&self) -> Result<String> {
        if self.vnode.inode_type() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.vnode.read_link()
    }

    /// Write symbolic link.
    pub fn write_link(&self, target: &str) -> Result<()> {
        if self.vnode.inode_type() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.vnode.write_link(target)
    }

    /// Rename a dentry to the new dentry by renaming inode.
    pub fn rename(&self, old_name: &str, new_dir: &Arc<Self>, new_name: &str) -> Result<()> {
        if old_name == "." || old_name == ".." || new_name == "." || new_name == ".." {
            return_errno_with_message!(Errno::EISDIR, "old_name or new_name is a directory");
        }
        if self.vnode.inode_type() != InodeType::Dir || new_dir.vnode.inode_type() != InodeType::Dir
        {
            return_errno!(Errno::ENOTDIR);
        }

        // Self and new_dir are same Dentry, just modify name
        if Arc::ptr_eq(&self.this(), new_dir) {
            if old_name == new_name {
                return Ok(());
            }
            let mut children = self.children.lock();
            self.vnode.rename(old_name, &self.vnode, new_name)?;
            match children.find_dentry(old_name) {
                Some(dentry) => {
                    children.delete_dentry(old_name);
                    dentry.set_name_and_parent(new_name, Some(self.this()));
                    children.insert_dentry(&dentry);
                }
                None => {
                    children.delete_dentry(new_name);
                }
            }
        } else {
            // Self and new_dir are different Dentry
            let (mut self_children, mut new_dir_children) =
                write_lock_children_on_two_dentries(&self, &new_dir);
            self.vnode.rename(old_name, &new_dir.vnode, new_name)?;
            match self_children.find_dentry(old_name) {
                Some(dentry) => {
                    self_children.delete_dentry(old_name);
                    dentry.set_name_and_parent(new_name, Some(new_dir.this()));
                    new_dir_children.insert_dentry(&dentry);
                }
                None => {
                    new_dir_children.delete_dentry(new_name);
                }
            }
        }
        Ok(())
    }

    /// Get the inode metadata
    pub fn inode_metadata(&self) -> Metadata {
        self.vnode.metadata()
    }

    /// Get the inode type
    pub fn inode_type(&self) -> InodeType {
        self.vnode.inode_type()
    }

    /// Get the inode permission mode
    pub fn inode_mode(&self) -> InodeMode {
        self.vnode.inode_mode()
    }

    /// Get the inode length
    pub fn inode_len(&self) -> usize {
        self.vnode.len()
    }

    /// Get the access timestamp
    pub fn atime(&self) -> Duration {
        self.vnode.atime()
    }

    /// Set the access timestamp
    pub fn set_atime(&self, time: Duration) {
        self.vnode.set_atime(time)
    }

    /// Get the modified timestamp
    pub fn mtime(&self) -> Duration {
        self.vnode.mtime()
    }

    /// Set the modified timestamp
    pub fn set_mtime(&self, time: Duration) {
        self.vnode.set_mtime(time)
    }

    /// Get the absolute path.
    pub fn abs_path(&self) -> String {
        let mut path = self.name();
        let mut dentry = self.this();

        loop {
            match dentry.parent() {
                None => break,
                Some(parent_dentry) => {
                    path = {
                        let parent_name = parent_dentry.name();
                        if parent_name != "/" {
                            parent_name + "/" + &path
                        } else {
                            parent_name + &path
                        }
                    };
                    dentry = parent_dentry;
                }
            }
        }

        debug_assert!(path.starts_with("/"));
        path
    }
}

struct Children {
    inner: BTreeMap<String, Weak<Dentry>>,
}

impl Children {
    pub fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    pub fn insert_dentry(&mut self, dentry: &Arc<Dentry>) {
        if dentry.vnode().is_dentry_cacheable() {
            DCACHE.lock().insert(dentry.key(), dentry.clone());
        }
        self.inner.insert(dentry.name(), Arc::downgrade(dentry));
    }

    pub fn delete_dentry(&mut self, name: &str) -> Option<Arc<Dentry>> {
        self.inner
            .remove(name)
            .and_then(|d| d.upgrade())
            .and_then(|d| DCACHE.lock().remove(&d.key()))
    }

    pub fn find_dentry(&mut self, name: &str) -> Option<Arc<Dentry>> {
        if let Some(dentry) = self.inner.get(name) {
            dentry.upgrade().or_else(|| {
                self.inner.remove(name);
                None
            })
        } else {
            None
        }
    }
}

#[derive(Clone, Hash, PartialOrd, Ord, Eq, PartialEq)]
struct DentryKey {
    name: String,
    parent_ptr: usize,
}

impl DentryKey {
    pub fn new(name: &str, parent: &Arc<Dentry>) -> Self {
        Self {
            name: String::from(name),
            parent_ptr: Arc::as_ptr(parent) as usize,
        }
    }
}

fn write_lock_children_on_two_dentries<'a>(
    this: &'a Dentry,
    other: &'a Dentry,
) -> (MutexGuard<'a, Children>, MutexGuard<'a, Children>) {
    let this_key = this.key();
    let other_key = other.key();
    if this_key < other_key {
        let this = this.children.lock();
        let other = other.children.lock();
        (this, other)
    } else {
        let other = other.children.lock();
        let this = this.children.lock();
        (this, other)
    }
}
