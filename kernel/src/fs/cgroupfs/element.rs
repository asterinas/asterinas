// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::String, sync::Arc};
use core::fmt::{self, Formatter};

use super::interfaces::DataProvider; // Use trait from the renamed module
use crate::{fs::utils::Inode, prelude::*};

/// Represents a cgroupfs directory.
#[derive(Debug)]
pub struct CgroupDir {
    pub(super) children: BTreeMap<String, Arc<dyn Inode>>, // Made pub(super) for access from inode.rs
}

/// Represents a cgroupfs symbolic link.
#[derive(Debug)]
pub struct CgroupSymlink {
    target_path: String,
}

impl CgroupSymlink {
    pub fn new(target_path: String) -> Self {
        CgroupSymlink { target_path }
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

/// Represents a cgroupfs file attribute.
pub struct CgroupAttr {
    data: Option<Box<dyn DataProvider>>,
}

impl CgroupAttr {
    pub fn new(data: Option<Box<dyn DataProvider>>) -> Self {
        CgroupAttr { data }
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
}

impl Default for CgroupAttr {
    fn default() -> Self {
        CgroupAttr::new(None)
    }
}

impl fmt::Debug for CgroupAttr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "CgroupAttr")
    }
}

/// The union of all cgroupfs element types.
#[derive(Debug)]
pub enum CgroupElement {
    Dir(CgroupDir),
    Symlink(CgroupSymlink),
    Attr(CgroupAttr),
}

impl CgroupElement {
    /// Creates a new `CgroupDir`.
    pub fn new_dir() -> Self {
        CgroupElement::Dir(CgroupDir {
            children: BTreeMap::new(),
        })
    }

    /// Creates a new `CgroupSymlink` with the given target path.
    pub fn new_symlink(target_path: &str) -> Self {
        CgroupElement::Symlink(CgroupSymlink::new(target_path.to_string()))
    }

    /// Creates a new `CgroupAttr` with the given data provider.
    pub fn new_attr(data: Option<Box<dyn DataProvider>>) -> Self {
        CgroupElement::Attr(CgroupAttr::new(data))
    }

    /// Sets the data provider for the cgroup attribute.
    pub fn set_data(&mut self, data: Box<dyn DataProvider>) -> Result<()> {
        match self {
            CgroupElement::Attr(ref mut attr) => {
                attr.set_data(data);
                Ok(())
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }

    /// Reads data from the cgroup attribute at the specified offset.
    pub fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        match self {
            CgroupElement::Attr(attr) => attr.read_at(offset, buf),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    /// Writes data to the cgroup attribute at the specified offset.
    pub fn write_at(&mut self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        match self {
            CgroupElement::Attr(attr) => attr.write_at(offset, buf),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    /// Removes a child from the directory.
    pub fn remove(&mut self, name: &str) -> Result<Arc<dyn Inode>> {
        let node = match self {
            CgroupElement::Dir(dir) => {
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
            CgroupElement::Dir(dir) => {
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
            CgroupElement::Dir(dir) => match dir.children.get(name) {
                Some(node) => Ok(node.clone() as Arc<dyn Inode>),
                None => return_errno!(Errno::ENOENT),
            },
            _ => return_errno!(Errno::ENOTDIR),
        }
    }

    /// Gets the children of the directory.
    pub fn get_children(&self) -> Option<&BTreeMap<String, Arc<dyn Inode>>> {
        match self {
            CgroupElement::Dir(dir) => Some(&dir.children),
            _ => None,
        }
    }

    /// Reads the target path of the symbolic link.
    pub fn read_link(&self) -> Result<String> {
        match self {
            CgroupElement::Symlink(link) => Ok(link.target_path()),
            _ => return_errno!(Errno::EINVAL),
        }
    }

    /// Writes the target path to the symbolic link.
    pub fn write_link(&mut self, target_path: &str) -> Result<()> {
        match self {
            CgroupElement::Symlink(link) => {
                link.set_target_path(target_path.to_string());
                Ok(())
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }
}
