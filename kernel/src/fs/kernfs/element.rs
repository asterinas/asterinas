// SPDX-License-Identifier: MPL-2.0

use alloc::string::String;
use core::fmt::{self, Formatter};

use super::DataProvider;
use crate::{fs::utils::Inode, prelude::*};

/// Represents a pseudo directory.
#[derive(Debug)]
pub struct PseudoDir {
    children: BTreeMap<String, Arc<dyn Inode>>,
}

/// Represents a pseudo symbolic link.
#[derive(Debug)]
pub struct PseudoSymlink {
    target_path: String,
}

impl PseudoSymlink {
    pub fn new(target_path: String) -> Self {
        PseudoSymlink {
            target_path: target_path,
        }
    }

    /// Gets the target path of the symbolic link.
    pub fn target_path(&self) -> String {
        self.target_path.clone()
    }

    /// Sets the target path of the symbolic link.
    pub fn set_target_path(&mut self, target_path: String) {
        self.target_path = target_path;
    }
}

/// Represents a pseudo file with data content.
/// DataProvider should be implemented for reading and writing data.
pub struct PseudoAttr {
    data: Option<Box<dyn DataProvider>>,
}

impl PseudoAttr {
    pub fn new(data: Option<Box<dyn DataProvider>>) -> Self {
        PseudoAttr { data }
    }

    pub fn set_data(&mut self, data: Box<dyn DataProvider>) {
        self.data = Some(data);
    }

    pub fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        if let Some(data) = &self.data {
            data.read_at(offset, buf)
        } else {
            Ok(0)
        }
    }

    pub fn write_at(&mut self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        if let Some(data) = &mut self.data {
            let new_size = data.write_at(offset, buf)?;
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
        PseudoAttr::new(None)
    }
}

impl fmt::Debug for PseudoAttr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "PseudoAttr")
    }
}

/// The union of all pseudo elements.
/// It can be a directory, a symbolic link, or a file with data content.
#[derive(Debug)]
pub enum PseudoElement {
    Dir(PseudoDir),
    Symlink(PseudoSymlink),
    Attr(PseudoAttr),
}

impl PseudoElement {
    /// Creates a new `PseudoDir`.
    pub fn new_dir() -> Self {
        PseudoElement::Dir(PseudoDir {
            children: BTreeMap::new(),
        })
    }

    /// Creates a new `PseudoSymlink` with the given target path.
    pub fn new_symlink(target_path: &str) -> Self {
        PseudoElement::Symlink(PseudoSymlink::new(target_path.to_string()))
    }

    /// Creates a new `PseudoAttr` with the given data provider.
    pub fn new_attr(data: Option<Box<dyn DataProvider>>) -> Self {
        PseudoElement::Attr(PseudoAttr::new(data))
    }

    /// Sets the data provider for the pseudo attribute.
    pub fn set_data(&mut self, data: Box<dyn DataProvider>) -> Result<()> {
        match self {
            PseudoElement::Attr(ref mut attr) => {
                attr.set_data(data);
                Ok(())
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }

    /// Reads data from the pseudo attribute at the specified offset.
    pub fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        match self {
            PseudoElement::Attr(attr) => attr.read_at(offset, buf),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    /// Writes data to the pseudo attribute at the specified offset.
    pub fn write_at(&mut self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        match self {
            PseudoElement::Attr(attr) => attr.write_at(offset, buf),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    /// Removes a child from the directory.
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

    /// Inserts a child into the directory.
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

    /// Looks up a child by name in the directory.
    pub fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        match self {
            PseudoElement::Dir(dir) => match dir.children.get(name) {
                Some(node) => Ok(node.clone() as Arc<dyn Inode>),
                None => return_errno!(Errno::ENOENT),
            },
            _ => return_errno!(Errno::ENOTDIR),
        }
    }

    /// Gets the children of the directory.
    pub fn get_children(&self) -> Option<BTreeMap<String, Arc<dyn Inode>>> {
        match self {
            PseudoElement::Dir(dir) => Some(dir.children.clone()),
            _ => None,
        }
    }

    /// Reads the target path of the symbolic link.
    pub fn read_link(&self) -> Result<String> {
        match self {
            PseudoElement::Symlink(link) => Ok(link.target_path()),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    /// Writes the target path to the symbolic link.
    pub fn write_link(&mut self, target_path: &str) -> Result<()> {
        match self {
            PseudoElement::Symlink(link) => {
                link.set_target_path(target_path.to_string());
                Ok(())
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }
}
