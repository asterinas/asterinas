use crate::prelude::*;
use alloc::string::String;

use super::{InodeMode, InodeType, Vnode, NAME_MAX};

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

    fn name(&self) -> String {
        self.inner.read().name.clone()
    }

    fn this(&self) -> Arc<Dentry> {
        self.inner.read().this.upgrade().unwrap()
    }

    fn parent(&self) -> Option<Arc<Dentry>> {
        self.inner
            .read()
            .parent
            .as_ref()
            .map(|p| p.upgrade().unwrap())
    }

    pub fn vnode(&self) -> &Vnode {
        &self.vnode
    }

    pub fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<Self>> {
        if self.vnode.inode().metadata().type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut inner = self.inner.write();
        if inner.children.get(name).is_some() {
            return_errno!(Errno::EEXIST);
        }
        let child = {
            let vnode = Vnode::new(self.vnode.inode().mknod(name, type_, mode)?)?;
            Dentry::new(name, vnode, Some(inner.this.clone()))
        };
        inner.children.insert(String::from(name), child.clone());
        Ok(child)
    }

    pub fn lookup(&self, name: &str) -> Result<Arc<Self>> {
        if self.vnode.inode().metadata().type_ != InodeType::Dir {
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
                    let vnode = Vnode::new(self.vnode.inode().lookup(name)?)?;
                    let dentry = Dentry::new(name, vnode, Some(inner.this.clone()));
                    inner.children.insert(String::from(name), dentry.clone());
                    dentry
                }
            }
        };
        Ok(dentry)
    }

    pub fn link(&self, old: &Arc<Self>, name: &str) -> Result<()> {
        if self.vnode.inode().metadata().type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut inner = self.inner.write();
        if inner.children.get(name).is_some() {
            return_errno!(Errno::EEXIST);
        }
        let target_vnode = old.vnode();
        self.vnode.inode().link(target_vnode.inode(), name)?;
        let new_dentry = Self::new(name, target_vnode.clone(), Some(inner.this.clone()));
        inner.children.insert(String::from(name), new_dentry);
        Ok(())
    }

    pub fn unlink(&self, name: &str) -> Result<()> {
        if self.vnode.inode().metadata().type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut inner = self.inner.write();
        self.vnode.inode().unlink(name)?;
        inner.children.remove(name);
        Ok(())
    }

    pub fn rmdir(&self, name: &str) -> Result<()> {
        if self.vnode.inode().metadata().type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let mut inner = self.inner.write();
        self.vnode.inode().rmdir(name)?;
        inner.children.remove(name);
        Ok(())
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
