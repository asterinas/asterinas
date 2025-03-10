// SPDX-License-Identifier: MPL-2.0

use alloc::string::String;
use core::fmt::{self, Formatter};

use super::DataProvider;
use crate::{fs::utils::Inode, prelude::*};

#[derive(Debug)]
pub struct PseudoDir {
    children: BTreeMap<String, Arc<dyn Inode>>,
}

#[derive(Debug)]
pub struct PseudoSymlink {
    target_path: String,
}

impl PseudoSymlink {
    pub fn new(target_kn: String) -> Self {
        PseudoSymlink {
            target_path: target_kn,
        }
    }

    pub fn target_path(&self) -> String {
        self.target_path.clone()
    }

    pub fn set_target_path(&mut self, target_kn: String) {
        self.target_path = target_kn;
    }
}

pub struct PseudoAttr {
    data: Option<Box<dyn DataProvider>>,
}

impl PseudoAttr {
    pub fn new() -> Self {
        PseudoAttr { data: None }
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

    pub fn truncate(&mut self, new_size: usize) -> Result<()> {
        if let Some(data) = &mut self.data {
            data.truncate(new_size)
        } else {
            Ok(())
        }
    }
}

impl Default for PseudoAttr {
    fn default() -> Self {
        PseudoAttr::new()
    }
}

impl fmt::Debug for PseudoAttr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "PseudoAttr")
    }
}

#[derive(Debug)]
pub enum PseudoElement {
    Dir(PseudoDir),         // Directory node
    Symlink(PseudoSymlink), // Symbolic link
    Attr(PseudoAttr),       // File with data content
}

impl PseudoElement {
    pub fn new_dir() -> Self {
        PseudoElement::Dir(PseudoDir {
            children: BTreeMap::new(),
        })
    }

    pub fn new_symlink(target_kn: &str) -> Self {
        PseudoElement::Symlink(PseudoSymlink::new(target_kn.to_string()))
    }

    pub fn new_attr() -> Self {
        PseudoElement::Attr(PseudoAttr::new())
    }

    pub fn set_data(&mut self, data: Box<dyn DataProvider>) -> Result<()> {
        match self {
            PseudoElement::Attr(ref mut attr) => {
                attr.set_data(data);
                Ok(())
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }

    pub fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        match self {
            PseudoElement::Attr(attr) => attr.read_at(offset, buf),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    pub fn write_at(&mut self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        match self {
            PseudoElement::Attr(attr) => attr.write_at(offset, buf),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    pub fn remove(&mut self, name: &str) -> Result<Arc<dyn Inode>> {
        let node = match self {
            PseudoElement::Dir(dir) => {
                if let Some(node) = dir.children.remove(name) {
                    node
                } else {
                    return_errno_with_message!(Errno::ENOENT, "no such file or directory");
                }
            }
            _ => return_errno_with_message!(Errno::ENOTDIR, "not a directory"),
        };
        Ok(node)
    }

    pub fn insert(&mut self, name: String, node: Arc<dyn Inode>) -> Result<()> {
        match self {
            PseudoElement::Dir(dir) => {
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
            PseudoElement::Dir(dir) => match dir.children.get(name) {
                Some(node) => Ok(node.clone() as Arc<dyn Inode>),
                None => return_errno!(Errno::ENOENT),
            },
            _ => return_errno!(Errno::ENOTDIR),
        }
    }

    pub fn get_children(&self) -> Option<BTreeMap<String, Arc<dyn Inode>>> {
        match self {
            PseudoElement::Dir(dir) => Some(dir.children.clone()),
            _ => None,
        }
    }

    pub fn read_link(&self) -> Result<String> {
        match self {
            PseudoElement::Symlink(link) => Ok(link.target_path()),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    pub fn write_link(&mut self, target_kn: &str) -> Result<()> {
        match self {
            PseudoElement::Symlink(link) => {
                link.set_target_path(target_kn.to_string());
                Ok(())
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }
}
