// SPDX-License-Identifier: MPL-2.0
#![allow(unused)]
use alloc::string::String;
use core::{
    fmt::{self, Formatter},
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    time::Duration,
};

use ostd::sync::{RwLock, RwMutex, RwMutexReadGuard};

use super::KernfsNode;
use crate::{
    events::{Events, Observer, Subject},
    fs::utils::{
        DirentVisitor, FileSystem, FsFlags, Inode, InodeMode, InodeType, Metadata, SuperBlock,
        NAME_MAX,
    },
    prelude::*,
    process::{Gid, Uid},
};

#[derive(Debug)]
pub struct KernfsElemDir {
    children: BTreeMap<String, Arc<dyn Inode>>,
}

#[derive(Debug)]
pub struct KernfsElemSymlink {
    target_kn: Weak<dyn Inode>,
}

impl KernfsElemSymlink {
    pub fn new(target_kn: Weak<dyn Inode>) -> Self {
        KernfsElemSymlink { target_kn }
    }

    pub fn target_kn(&self) -> Option<Arc<dyn Inode>> {
        self.target_kn.upgrade()
    }
}

pub trait DataProvider: Any + Sync + Send {
    fn read_at(&self, writer: &mut VmWriter, offset: usize) -> Result<usize>;
    fn write_at(&mut self, reader: &mut VmReader, offset: usize) -> Result<usize>;
}

pub struct KernfsElemAttr {
    data: Option<Box<dyn DataProvider>>,
}

impl KernfsElemAttr {
    pub fn new() -> Self {
        KernfsElemAttr { data: None }
    }

    pub fn set_data(&mut self, data: Box<dyn DataProvider>) {
        self.data = Some(data);
    }

    pub fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        if let Some(data) = &self.data {
            data.read_at(buf, offset)
        } else {
            Ok(0)
        }
    }

    pub fn write_at(&mut self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        if let Some(data) = &mut self.data {
            let new_size = data.write_at(buf, offset)?;
            Ok(new_size)
        } else {
            Ok(0)
        }
    }
}

impl Default for KernfsElemAttr {
    fn default() -> Self {
        KernfsElemAttr::new()
    }
}

impl fmt::Debug for KernfsElemAttr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "KernfsElemAttr")
    }
}

#[derive(Debug)]
pub enum KernfsElem {
    Dir(KernfsElemDir),
    Symlink(KernfsElemSymlink),
    Attr(KernfsElemAttr),
}

impl KernfsElem {
    pub fn new_dir() -> Self {
        KernfsElem::Dir(KernfsElemDir {
            children: BTreeMap::new(),
        })
    }

    pub fn new_symlink(target_kn: Weak<dyn Inode>) -> Self {
        KernfsElem::Symlink(KernfsElemSymlink::new(target_kn))
    }

    pub fn new_attr() -> Self {
        KernfsElem::Attr(KernfsElemAttr::new())
    }

    pub fn is_dir(&self) -> bool {
        matches!(self, KernfsElem::Dir(_))
    }

    pub fn is_symlink(&self) -> bool {
        matches!(self, KernfsElem::Symlink(_))
    }

    pub fn is_attr(&self) -> bool {
        matches!(self, KernfsElem::Attr(_))
    }

    pub fn set_data(&mut self, data: Box<dyn DataProvider>) -> Result<()> {
        match self {
            KernfsElem::Attr(ref mut attr) => {
                attr.set_data(data);
                Ok(())
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }

    pub fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        match self {
            KernfsElem::Attr(attr) => attr.read_at(offset, buf),
            KernfsElem::Symlink(link) => {
                if let Some(target_kn) = link.target_kn() {
                    target_kn.read_at(offset, buf)
                } else {
                    return_errno!(Errno::ENOENT)
                }
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }

    pub fn write_at(&mut self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        match self {
            KernfsElem::Attr(attr) => attr.write_at(offset, buf),
            KernfsElem::Symlink(link) => {
                if let Some(target_kn) = link.target_kn() {
                    target_kn.write_at(offset, buf)
                } else {
                    return_errno!(Errno::ENOENT)
                }
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }

    pub fn remove(&mut self, name: &str) -> Result<()> {
        match self {
            KernfsElem::Dir(dir) => {
                if dir.children.remove(name).is_none() {
                    return_errno_with_message!(Errno::ENOENT, "no such file or directory");
                }
            }
            _ => return_errno_with_message!(Errno::ENOTDIR, "not a directory"),
        }
        Ok(())
    }

    pub fn insert(&mut self, name: String, node: Arc<dyn Inode>) -> Result<()> {
        match self {
            KernfsElem::Dir(dir) => {
                if dir.children.contains_key(&name)
                    || dir.children.insert(name.clone(), node.clone()).is_some()
                {
                    return_errno_with_message!(Errno::EEXIST, "file exists");
                }
            }
            _ => return_errno_with_message!(Errno::ENOTDIR, "not a directory"),
        }
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        match self {
            KernfsElem::Dir(dir) => match dir.children.get(name) {
                Some(node) => Ok(node.clone()),
                None => return_errno!(Errno::ENOENT),
            },
            _ => return_errno!(Errno::ENOTDIR),
        }
    }

    pub fn get_children(&self) -> Option<BTreeMap<String, Arc<dyn Inode>>> {
        match self {
            KernfsElem::Dir(dir) => Some(dir.children.clone()),
            _ => None,
        }
    }
}
