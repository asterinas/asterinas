use crate::prelude::*;
use alloc::string::String;

use super::{InodeMode, InodeType, NAME_MAX};
use crate::fs::vfs_inode::VfsInode;

pub struct Dentry {
    inner: RwLock<Dentry_>,
    inode: VfsInode,
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
    pub fn new_root(inode: VfsInode) -> Arc<Self> {
        let root = Self::new("/", inode, None);
        root
    }

    /// Internal constructor
    fn new(name: &str, inode: VfsInode, parent: Option<Weak<Dentry>>) -> Arc<Self> {
        let dentry = {
            let inner = RwLock::new(Dentry_::new(name, parent));
            Arc::new(Self { inner, inode })
        };
        dentry.inner.write().this = Arc::downgrade(&dentry);
        dentry
    }

    fn name(&self) -> String {
        self.inner.read().name.clone()
    }

    pub fn inode(&self) -> &VfsInode {
        &self.inode
    }

    pub fn create_child(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<Self>> {
        let mut inner = self.inner.write();
        let child = {
            let inode = VfsInode::new(self.inode().raw_inode().mknod(name, type_, mode)?)?;
            Dentry::new(name, inode, Some(inner.this.clone()))
        };
        inner.children.insert(String::from(name), child.clone());
        Ok(child)
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

    pub fn get(&self, name: &str) -> Result<Arc<Dentry>> {
        if self.inode.raw_inode().metadata().type_ != InodeType::Dir {
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
                    let inode = VfsInode::new(self.inode.raw_inode().lookup(name)?)?;
                    let dentry = Dentry::new(name, inode, Some(inner.this.clone()));
                    inner.children.insert(String::from(name), dentry.clone());
                    dentry
                }
            }
        };
        Ok(dentry)
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
