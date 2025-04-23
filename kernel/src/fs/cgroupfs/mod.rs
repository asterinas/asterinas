// SPDX-License-Identifier: MPL-2.0
#![allow(unused)]

mod element;
mod inode;
mod interfaces;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::{Arc, Weak},
};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;

use self::{
    inode::CgroupINode,
    interfaces::{CgroupExt, DataProvider},
};
use crate::{
    fs::utils::{FileSystem, FsFlags, Inode, InodeMode, InodeType, SuperBlock, NAME_MAX},
    prelude::*,
    util::{MultiRead, MultiWrite},
};

#[derive(Debug)]
struct CgroupDataProvider {
    content: Option<String>, // None for placeholder/dynamic files, Some(String) for static files
    writable: bool,
}

impl CgroupDataProvider {
    // Constructor for static, read-only files
    fn new_static(content: &str) -> Self {
        Self {
            content: Some(content.to_string()),
            writable: false,
        }
    }

    // Constructor for placeholder/dynamic files (potentially writable)
    fn new_dynamic(writable: bool) -> Self {
        Self {
            content: None,
            writable,
        }
    }
}

impl DataProvider for CgroupDataProvider {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let content_bytes = match &self.content {
            Some(s) => s.as_bytes(),
            None => b"",
        };

        if offset >= content_bytes.len() {
            return Ok(0);
        }
        let len_to_read = content_bytes.len() - offset;
        let written = writer.write(&mut (&content_bytes[offset..offset + len_to_read]).into())?;
        Ok(written)
    }

    fn write_at(&mut self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if !self.writable {
            return_errno!(Errno::EPERM);
        }

        if self.content.is_none() {
            self.content = Some(String::new());
        }

        let mut content = self.content.as_mut().unwrap();
        let mut buf = [0u8; 128];
        let mut total_read = 0;

        loop {
            let mut writer = VmWriter::from(&mut buf[..]);
            let read_result = reader.read(&mut writer);

            match read_result {
                Ok(0) => break,
                Ok(n) => {
                    total_read += n;
                    content.push_str(String::from_utf8_lossy(&buf[..n]).as_ref());
                }
                Err(e) => return Err(e),
            }
        }

        Ok(total_read)
    }
}

#[derive(Debug)]
struct CgroupFsExt;

impl CgroupFsExt {
    // Helper to create standard files within a new cgroup directory
    fn populate_cgroup_dir(dir_node: Arc<CgroupINode>) -> Result<()> {
        // Use the same file creation logic as for the root, but relative to dir_node
        // Note: Permissions might differ slightly for non-root cgroups in real systems.
        // Note: The DataProviders created here should ideally be linked to the
        //       specific cgroup represented by dir_node.

        // --- Read-only files ---
        CgroupFs::create_file(
            "cgroup.controllers",
            0o444,
            dir_node.clone(),
            Box::new(CgroupDataProvider::new_static("")),
            None,
        )?;
        CgroupFs::create_file(
            "cgroup.stat",
            0o444,
            dir_node.clone(),
            Box::new(CgroupDataProvider::new_static(
                "nr_descendants 0\nnr_dying_descendants 0\n",
            )),
            None,
        )?;
        CgroupFs::create_file(
            "cgroup.events",
            0o444,
            dir_node.clone(),
            Box::new(CgroupDataProvider::new_static("")),
            None,
        )?;
        CgroupFs::create_file(
            "cgroup.max.depth",
            0o444,
            dir_node.clone(),
            Box::new(CgroupDataProvider::new_static("max\n")),
            None,
        )?;
        CgroupFs::create_file(
            "cgroup.max.descendants",
            0o444,
            dir_node.clone(),
            Box::new(CgroupDataProvider::new_static("max\n")),
            None,
        )?;

        // --- Read/Write files ---
        CgroupFs::create_file(
            "cgroup.subtree_control",
            0o644,
            dir_node.clone(),
            Box::new(CgroupDataProvider::new_dynamic(true)),
            None,
        )?;
        CgroupFs::create_file(
            "cgroup.procs",
            0o644,
            dir_node.clone(),
            Box::new(CgroupDataProvider::new_dynamic(true)),
            None,
        )?;
        CgroupFs::create_file(
            "cgroup.threads",
            0o644,
            dir_node.clone(),
            Box::new(CgroupDataProvider::new_dynamic(true)),
            None,
        )?;

        Ok(())
    }
}

impl CgroupExt for CgroupFsExt {
    fn on_create(&self, name: &str, node: Arc<dyn Inode>) -> Result<()> {
        debug!(
            "CgroupFsExt::on_create: name={}, type={:?}",
            name,
            node.type_()
        );
        if node.type_() == InodeType::Dir {
            let dir_node = node
                .downcast_ref::<CgroupINode>()
                .ok_or_else(|| Error::new(Errno::EINVAL))?
                .this();

            Self::populate_cgroup_dir(dir_node)?;
        }
        Ok(())
    }

    fn on_remove(&self, name: &str) -> Result<()> {
        debug!("CgroupFsExt::on_remove: name={}", name);
        // TODO:
        // 1. Check if the cgroup associated with 'name' is empty (no processes, no children).
        // 2. If empty, remove the kernel cgroup object.
        // 3. If not empty, return EBUSY or appropriate error.
        // For now, allow removal (kernfs handles the VFS part).
        Ok(())
    }
}

// --- CgroupFs Implementation ---

// Magic number for cgroupfs v2 (taken from Linux)
const CGROUP2_SUPER_MAGIC: u64 = 0x63677270;
const CGROUP_ROOT_INO: u64 = 1;
const BLOCK_SIZE_CGROUP: usize = 4096;

pub static CGROUPFS_REF: Once<Arc<CgroupFs>> = Once::new();

pub struct CgroupFs {
    sb: SuperBlock,
    root: Arc<CgroupINode>,
    inode_allocator: AtomicU64,
    extension: Arc<dyn CgroupExt>,
    this: Weak<Self>,
}

impl CgroupFs {
    pub fn new() -> Arc<Self> {
        let cgroup_ext = Arc::new(CgroupFsExt);

        let fs: Arc<CgroupFs> = Arc::new_cyclic(|weak_fs: &Weak<CgroupFs>| {
            let root_inode = CgroupINode::new_root(
                weak_fs.clone(),
                CGROUP_ROOT_INO,
                BLOCK_SIZE_CGROUP,
                Some(cgroup_ext.clone()),
            );

            Self {
                sb: SuperBlock::new(CGROUP2_SUPER_MAGIC, BLOCK_SIZE_CGROUP, NAME_MAX),
                root: root_inode,
                inode_allocator: AtomicU64::new(CGROUP_ROOT_INO + 1),
                extension: cgroup_ext,
                this: weak_fs.clone(),
            }
        });
        CGROUPFS_REF.call_once(|| fs.clone());

        CgroupFsExt::populate_cgroup_dir(fs.root.clone())
            .expect("Failed to create cgroup root files");

        fs
    }

    pub fn create_file(
        name: &str,
        mode: u16,
        parent: Arc<CgroupINode>,
        data_provider: Box<dyn DataProvider>,
        extension: Option<Arc<dyn CgroupExt>>,
    ) -> Result<Arc<CgroupINode>> {
        let mode = InodeMode::from_bits_truncate(mode);
        CgroupINode::new_attr(
            name,
            Some(mode),
            Some(data_provider),
            extension,
            parent.this_weak(),
        )
    }

    pub fn create_dir(
        name: &str,
        mode: u16,
        parent: Arc<CgroupINode>,
        extension: Option<Arc<dyn CgroupExt>>,
    ) -> Result<Arc<CgroupINode>> {
        let mode = InodeMode::from_bits_truncate(mode);
        let ext_to_use = extension.or_else(|| {
            parent
                .fs()
                .downcast_ref::<CgroupFs>()
                .map(|fs| fs.extension.clone())
        });
        CgroupINode::new_dir(name, Some(mode), ext_to_use, parent.this_weak())
    }

    pub fn init_parent_dirs(&self, parent: &str) -> Result<Arc<CgroupINode>> {
        let mut current_node = self.root.clone();
        for dir_name in parent.split('/').filter(|s| !s.is_empty()) {
            match current_node.lookup(dir_name) {
                Ok(next_inode) => {
                    current_node = next_inode
                        .downcast_ref::<CgroupINode>()
                        .ok_or_else(|| Error::new(Errno::ENOTDIR))?
                        .this();
                }
                Err(e) if e.error() == Errno::ENOENT => {
                    current_node = Self::create_dir(
                        dir_name,
                        0o755,
                        current_node.clone(),
                        Some(self.extension.clone()),
                    )?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(current_node)
    }

    pub fn alloc_unique_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::SeqCst)
    }
}

impl FileSystem for CgroupFs {
    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
    }
}
