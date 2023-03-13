use crate::prelude::*;
use alloc::string::String;
use spin::RwLockWriteGuard;

use super::{DeviceId, InodeMode, InodeType, Metadata, Vnode, NAME_MAX};

pub struct Dentry {
    inner: RwLock<Dentry_>,
    vnode: Vnode,
}

struct Dentry_ {
    name: String,
    this: Weak<Dentry>,
    parent: Option<Weak<Dentry>>,
    children: BTreeMap<String, Arc<Dentry>>,
}

impl Dentry_ {
    pub fn new(name: &str, parent: Option<Weak<Dentry>>) -> Self {
        Self {
            name: String::from(name),
            this: Weak::default(),
            parent,
            children: BTreeMap::new(),
        }
    }
}

impl Dentry {
    /// Create a new dentry cache with root inode
    pub fn new_root(root_vnode: Vnode) -> Arc<Self> {
        let root = Self::new("/", root_vnode, None);
        root
    }

    /// Internal constructor
    fn new(name: &str, vnode: Vnode, parent: Option<Weak<Dentry>>) -> Arc<Self> {
        let dentry = {
            let inner = RwLock::new(Dentry_::new(name, parent));
            Arc::new(Self { inner, vnode })
        };
        dentry.inner.write().this = Arc::downgrade(&dentry);
        dentry
    }

    pub fn name(&self) -> String {
        self.inner.read().name.clone()
    }

    fn set_name(&self, name: &str) {
        self.inner.write().name = String::from(name);
    }

    fn this(&self) -> Arc<Dentry> {
        self.inner.read().this.upgrade().unwrap()
    }

    pub fn parent(&self) -> Option<Arc<Dentry>> {
        self.inner
            .read()
            .parent
            .as_ref()
            .map(|p| p.upgrade().unwrap())
    }

    fn set_parent(&self, parent: &Arc<Dentry>) {
        self.inner.write().parent = Some(Arc::downgrade(parent));
    }

    pub(in crate::fs) fn vnode(&self) -> &Vnode {
        &self.vnode
    }

    pub fn mknod(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        dev: Option<DeviceId>,
    ) -> Result<Arc<Self>> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut inner = self.inner.write();
        if inner.children.get(name).is_some() {
            return_errno!(Errno::EEXIST);
        }
        let child = {
            let vnode = self.vnode.mknod(name, type_, mode, dev)?;
            Dentry::new(name, vnode, Some(inner.this.clone()))
        };
        inner.children.insert(String::from(name), child.clone());
        Ok(child)
    }

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
                let mut inner = self.inner.write();
                if let Some(dentry) = inner.children.get(name) {
                    dentry.clone()
                } else {
                    let vnode = self.vnode.lookup(name)?;
                    let dentry = Dentry::new(name, vnode, Some(inner.this.clone()));
                    inner.children.insert(String::from(name), dentry.clone());
                    dentry
                }
            }
        };
        Ok(dentry)
    }

    pub fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut inner = self.inner.write();
        if inner.children.get(name).is_some() {
            return_errno!(Errno::EEXIST);
        }
        let target_vnode = old.vnode();
        self.vnode.link(target_vnode, name)?;
        let new_dentry = Self::new(name, target_vnode.clone(), Some(inner.this.clone()));
        inner.children.insert(String::from(name), new_dentry);
        Ok(())
    }

    pub fn unlink(&self, name: &str) -> Result<()> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut inner = self.inner.write();
        self.vnode.unlink(name)?;
        inner.children.remove(name);
        Ok(())
    }

    pub fn rmdir(&self, name: &str) -> Result<()> {
        if self.vnode.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut inner = self.inner.write();
        self.vnode.rmdir(name)?;
        inner.children.remove(name);
        Ok(())
    }

    pub fn read_link(&self) -> Result<String> {
        if self.vnode.inode_type() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.vnode.read_link()
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        if self.vnode.inode_type() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.vnode.write_link(target)
    }

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
            let mut inner = self.inner.write();
            let dentry = if let Some(dentry) = inner.children.get(old_name) {
                dentry.clone()
            } else {
                let vnode = self.vnode.lookup(old_name)?;
                Dentry::new(old_name, vnode, Some(inner.this.clone()))
            };
            self.vnode.rename(old_name, &self.vnode, new_name)?;
            inner.children.remove(old_name);
            dentry.set_name(new_name);
            inner.children.insert(String::from(new_name), dentry);
        } else {
            // Self and new_dir are different Dentry
            let (mut self_inner, mut new_dir_inner) = write_lock_two_dentries(&self, &new_dir);
            let dentry = if let Some(dentry) = self_inner.children.get(old_name) {
                dentry.clone()
            } else {
                let vnode = self.vnode.lookup(old_name)?;
                Dentry::new(old_name, vnode, Some(self_inner.this.clone()))
            };
            self.vnode.rename(old_name, &new_dir.vnode, new_name)?;
            self_inner.children.remove(old_name);
            dentry.set_name(new_name);
            dentry.set_parent(&new_dir.this());
            new_dir_inner
                .children
                .insert(String::from(new_name), dentry);
        }
        Ok(())
    }

    pub fn inode_metadata(&self) -> Metadata {
        self.vnode.metadata()
    }

    pub fn inode_type(&self) -> InodeType {
        self.vnode.inode_type()
    }

    pub fn inode_mode(&self) -> InodeMode {
        self.vnode.inode_mode()
    }

    pub fn inode_len(&self) -> usize {
        self.vnode.len()
    }

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

fn write_lock_two_dentries<'a>(
    this: &'a Dentry,
    other: &'a Dentry,
) -> (RwLockWriteGuard<'a, Dentry_>, RwLockWriteGuard<'a, Dentry_>) {
    let this_ptr = Arc::as_ptr(&this.this());
    let other_ptr = Arc::as_ptr(&other.this());
    if this_ptr < other_ptr {
        let this = this.inner.write();
        let other = other.inner.write();
        (this, other)
    } else {
        let other = other.inner.write();
        let this = this.inner.write();
        (this, other)
    }
}
