// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use alloc::sync::Arc;

use hashbrown::HashMap;
use rand::{Rng, RngCore};

use super::{Inode, InodeMode, InodeType};
use crate::prelude::*;

pub struct FileInMemory {
    pub name: String,
    pub inode: Arc<dyn Inode>,
    pub valid_len: usize,
    pub contents: Vec<u8>,
}
pub struct DirInMemory {
    pub depth: u32,
    pub name: String,
    pub inode: Arc<dyn Inode>,
    pub sub_names: Vec<String>,
    pub sub_dirs: HashMap<String, DentryInMemory>,
}
pub enum DentryInMemory {
    File(FileInMemory),
    Dir(DirInMemory),
}
pub enum Operation {
    Read(usize, usize),
    Write(usize, usize),
    Resize(usize),
    Create(String, InodeType),
    Lookup(String),
    Readdir(),
    Unlink(String),
    Rmdir(String),
    Rename(String, String),
}

impl Operation {
    const CREATE_FILE_ID: usize = 0;
    const CREATE_DIR_ID: usize = 1;
    const UNLINK_ID: usize = 2;
    const RMDIR_ID: usize = 3;
    const LOOKUP_ID: usize = 4;
    const READDIR_ID: usize = 5;
    const RENAME_ID: usize = 6;
    const DIR_OP_NUM: usize = 7;
    const READ_ID: usize = 0;
    const WRITE_ID: usize = 1;
    const RESIZE_ID: usize = 2;
    const FILE_OP_NUM: usize = 3;
    const MAX_PAGE_PER_FILE: usize = 10;

    pub fn generate_random_dir_operation(
        dir: &mut DirInMemory,
        idx: u32,
        rng: &mut dyn RngCore,
    ) -> Self {
        let op_id = rng.gen_range(0..Self::DIR_OP_NUM);
        if op_id == Self::CREATE_FILE_ID {
            Operation::Create(idx.to_string(), InodeType::File)
        } else if op_id == Self::CREATE_DIR_ID {
            Operation::Create(idx.to_string(), InodeType::Dir)
        } else if op_id == Self::UNLINK_ID && !dir.sub_names.is_empty() {
            let rand_idx = rng.gen_range(0..dir.sub_names.len());
            let name = dir.sub_names[rand_idx].clone();
            Operation::Unlink(name)
        } else if op_id == Self::RMDIR_ID && !dir.sub_names.is_empty() {
            let rand_idx = rng.gen_range(0..dir.sub_names.len());
            let name = dir.sub_names[rand_idx].clone();
            Operation::Rmdir(name)
        } else if op_id == Self::LOOKUP_ID && !dir.sub_names.is_empty() {
            let rand_idx = rng.gen_range(0..dir.sub_names.len());
            let name = dir.sub_names[rand_idx].clone();
            Operation::Lookup(name)
        } else if op_id == Self::READDIR_ID {
            Operation::Readdir()
        } else if op_id == Self::RENAME_ID && !dir.sub_names.is_empty() {
            let rand_old_idx = rng.gen_range(0..dir.sub_names.len());
            let old_name = dir.sub_names[rand_old_idx].clone();
            let rename_to_an_exist = rng.gen_bool(0.5);
            if rename_to_an_exist {
                let rand_new_idx = rng.gen_range(0..dir.sub_names.len());
                let new_name = dir.sub_names[rand_new_idx].clone();
                Operation::Rename(old_name, new_name)
            } else {
                Operation::Rename(old_name, idx.to_string())
            }
        } else {
            Operation::Create(idx.to_string(), InodeType::File)
        }
    }

    pub fn generate_random_file_operation(
        file: &mut FileInMemory,
        idx: u32,
        rng: &mut dyn RngCore,
    ) -> Self {
        let op_id = rng.gen_range(0..Self::FILE_OP_NUM);
        if op_id == Self::READ_ID {
            let (offset, len) =
                generate_random_offset_len(Self::MAX_PAGE_PER_FILE * PAGE_SIZE, rng);
            Operation::Read(offset, len)
        } else if op_id == Self::WRITE_ID {
            let (offset, len) =
                generate_random_offset_len(Self::MAX_PAGE_PER_FILE * PAGE_SIZE, rng);
            Operation::Write(offset, len)
        } else if op_id == Self::RESIZE_ID {
            let pg_num = rng.gen_range(0..Self::MAX_PAGE_PER_FILE);
            let new_size = (pg_num * PAGE_SIZE).max(file.contents.len());
            Operation::Resize(new_size)
        } else {
            let valid_len = file.valid_len;
            Operation::Read(0, valid_len)
        }
    }
}

impl DirInMemory {
    pub fn remove_sub_names(&mut self, name: &String) {
        for idx in 0..self.sub_names.len() {
            if self.sub_names[idx].eq(name) {
                self.sub_names.remove(idx);
                break;
            }
        }
    }

    fn test_create(&mut self, name: &String, type_: InodeType) {
        info!(
            "Create: parent = {:?}, name = {:?}, type = {:?}",
            self.name, name, type_
        );

        let create_result = self.inode.create(name, type_, InodeMode::all());
        if self.sub_dirs.contains_key(name) {
            assert!(create_result.is_err());
            info!(
                "    create {:?}/{:?} failed: {:?}",
                self.name,
                name,
                create_result.unwrap_err()
            );
            return;
        }
        assert!(
            create_result.is_ok(),
            "Fail to create {:?}: {:?}",
            name,
            create_result.unwrap_err()
        );
        info!(
            "    create {:?}/{:?}({:?}) succeeded",
            self.name, name, type_
        );

        let new_dentry_in_mem = if type_ == InodeType::File {
            let file = FileInMemory {
                name: name.clone(),
                inode: create_result.unwrap(),
                valid_len: 0,
                contents: Vec::<u8>::new(),
            };
            DentryInMemory::File(file)
        } else {
            DentryInMemory::Dir(DirInMemory {
                depth: self.depth + 1,
                name: name.clone(),
                inode: create_result.unwrap(),
                sub_names: Vec::new(),
                sub_dirs: HashMap::new(),
            })
        };
        let _ = self.sub_dirs.insert(name.to_string(), new_dentry_in_mem);
        self.sub_names.push(name.to_string());
    }

    fn test_lookup(&self, name: &String) {
        info!("Lookup: parent = {:?}, name = {:?}", self.name, name);

        let lookup_result = self.inode.lookup(name);
        if self.sub_dirs.get(name).is_some() {
            assert!(
                lookup_result.is_ok(),
                "Fail to lookup {:?}: {:?}",
                name,
                lookup_result.unwrap_err()
            );
            info!("    lookup {:?}/{:?} succeeded", self.name, name);
        } else {
            assert!(lookup_result.is_err());
            info!(
                "    lookup {:?}/{:?} failed: {:?}",
                self.name,
                name,
                lookup_result.unwrap_err()
            );
        }
    }

    fn test_readdir(&mut self) {
        info!("Readdir: parent = {:?}", self.name);

        let mut sub: Vec<String> = Vec::new();
        let readdir_result = self.inode.readdir_at(0, &mut sub);
        assert!(readdir_result.is_ok(), "Fail to read directory",);
        assert!(readdir_result.unwrap() == self.sub_dirs.len() + 2);
        assert!(sub.len() == self.sub_dirs.len() + 2);

        // To remove "." and ".."
        sub.remove(0);
        sub.remove(0);
        sub.sort();
        self.sub_names.sort();
        for (i, name) in sub.iter().enumerate() {
            assert!(
                name.eq(&self.sub_names[i]),
                "Directory entry mismatch: read {:?} should be {:?}",
                name,
                self.sub_names[i]
            );
        }
    }

    fn test_unlink(&mut self, name: &String) {
        info!("Unlink: parent = {:?}, name = {:?}", self.name, name);

        let unlink_result = self.inode.unlink(name);
        if let Option::Some(sub) = self.sub_dirs.get(name)
            && let DentryInMemory::File(_) = sub
        {
            assert!(
                unlink_result.is_ok(),
                "Fail to remove file {:?}/{:?}: {:?}",
                self.name,
                name,
                unlink_result.unwrap_err()
            );
            info!("    unlink {:?}/{:?} succeeded", self.name, name);
            let _ = self.sub_dirs.remove(name);
            self.remove_sub_names(name);
        } else {
            assert!(unlink_result.is_err());
            info!(
                "    unlink {:?}/{:?} failed: {:?}",
                self.name,
                name,
                unlink_result.unwrap_err()
            );
        }
    }

    fn test_rmdir(&mut self, name: &String) {
        info!("Rmdir: parent = {:?}, name = {:?}", self.name, name);

        let rmdir_result = self.inode.rmdir(name);
        if let Option::Some(sub) = self.sub_dirs.get(name)
            && let DentryInMemory::Dir(sub_dir) = sub
            && sub_dir.sub_dirs.is_empty()
        {
            assert!(
                rmdir_result.is_ok(),
                "Fail to remove directory {:?}/{:?}: {:?}",
                self.name,
                name,
                rmdir_result.unwrap_err()
            );
            info!("    rmdir {:?}/{:?} succeeded", self.name, name);
            let _ = self.sub_dirs.remove(name);
            self.remove_sub_names(name);
        } else {
            assert!(rmdir_result.is_err());
            info!(
                "    rmdir {:?}/{:?} failed: {:?}",
                self.name,
                name,
                rmdir_result.unwrap_err()
            );
        }
    }

    fn test_rename(&mut self, old_name: &String, new_name: &String) {
        info!(
            "Rename: parent = {:?}, old_name = {:?}, target = {:?}, new_name = {:?}",
            self.name, old_name, self.name, new_name
        );

        let rename_result = self.inode.rename(old_name, &self.inode, new_name);
        if old_name.eq(new_name) {
            assert!(rename_result.is_ok());
            info!(
                "    rename {:?}/{:?} to {:?}/{:?} succeeded",
                self.name, old_name, self.name, new_name
            );
            return;
        }
        let mut valid_rename: bool = false;
        let mut exist: bool = false;
        if let Option::Some(old_sub) = self.sub_dirs.get(old_name) {
            let exist_new_sub = self.sub_dirs.get(new_name);
            match old_sub {
                DentryInMemory::File(old_file) => {
                    if let Option::Some(exist_new_sub_) = exist_new_sub
                        && let DentryInMemory::File(exist_new_file) = exist_new_sub_
                    {
                        valid_rename = true;
                        exist = true;
                    } else if exist_new_sub.is_none() {
                        valid_rename = true;
                    }
                }
                DentryInMemory::Dir(old_dir) => {
                    if let Option::Some(exist_new_sub_) = exist_new_sub
                        && let DentryInMemory::Dir(exist_new_dir) = exist_new_sub_
                        && exist_new_dir.sub_dirs.is_empty()
                    {
                        valid_rename = true;
                        exist = true;
                    } else if exist_new_sub.is_none() {
                        valid_rename = true;
                    }
                }
            }
        }
        if valid_rename {
            assert!(
                rename_result.is_ok(),
                "Fail to rename {:?}/{:?} to {:?}/{:?}: {:?}",
                self.name,
                old_name,
                self.name,
                new_name,
                rename_result.unwrap_err()
            );
            info!(
                "    rename {:?}/{:?} to {:?}/{:?} succeeded",
                self.name, old_name, self.name, new_name
            );
            let lookup_new_inode_result = self.inode.lookup(new_name);
            assert!(
                lookup_new_inode_result.is_ok(),
                "Fail to lookup new name {:?}: {:?}",
                new_name,
                lookup_new_inode_result.unwrap_err()
            );
            let mut old = self.sub_dirs.remove(old_name).unwrap();
            self.remove_sub_names(old_name);
            match old {
                DentryInMemory::Dir(ref mut dir) => {
                    dir.inode = lookup_new_inode_result.unwrap();
                    dir.name.clone_from(new_name);
                    dir.depth = self.depth + 1;
                }
                DentryInMemory::File(ref mut file) => {
                    file.inode = lookup_new_inode_result.unwrap();
                    file.name.clone_from(new_name);
                }
            }
            if exist {
                let _ = self.sub_dirs.remove(new_name);
                self.remove_sub_names(new_name);
            }
            self.sub_dirs.insert(new_name.to_string(), old);
            self.sub_names.push(new_name.to_string());
        } else {
            assert!(rename_result.is_err());
            info!(
                "    rename {:?}/{:?} to {:?}/{:?} failed: {:?}",
                self.name,
                old_name,
                self.name,
                new_name,
                rename_result.unwrap_err()
            );
        }
    }

    pub fn execute_and_test(&mut self, op: Operation) {
        match op {
            Operation::Create(name, type_) => self.test_create(&name, type_),
            Operation::Lookup(name) => self.test_lookup(&name),
            Operation::Readdir() => self.test_readdir(),
            Operation::Unlink(name) => self.test_unlink(&name),
            Operation::Rmdir(name) => self.test_rmdir(&name),
            Operation::Rename(old_name, new_name) => self.test_rename(&old_name, &new_name),
            _ => {}
        }
    }
}

impl FileInMemory {
    fn test_read(&self, offset: usize, len: usize) {
        info!(
            "Read: name = {:?}, offset = {:?}, len = {:?}",
            self.name, offset, len
        );
        let mut buf = vec![0; len];
        let read_result = self.inode.read_bytes_at(offset, &mut buf);
        assert!(
            read_result.is_ok(),
            "Fail to read file in range [{:?}, {:?}): {:?}",
            offset,
            offset + len,
            read_result.unwrap_err()
        );
        info!("    read succeeded");
        let (start, end) = (
            offset.min(self.valid_len),
            (offset + len).min(self.valid_len),
        );
        assert!(
            buf[..(end - start)].eq(&self.contents[start..end]),
            "Read file contents mismatch"
        );
    }

    fn test_write(&mut self, offset: usize, len: usize, rng: &mut dyn RngCore) {
        // Avoid holes in a file.
        let (write_start_offset, write_len) = if offset > self.valid_len {
            (self.valid_len, len + offset - self.valid_len)
        } else {
            (offset, len)
        };
        info!(
            "Write: name = {:?}, offset = {:?}, len = {:?}",
            self.name, write_start_offset, write_len
        );
        let mut buf = vec![0; write_len];
        rng.fill_bytes(&mut buf);
        let write_result = self.inode.write_bytes_at(write_start_offset, &buf);
        assert!(
            write_result.is_ok(),
            "Fail to write file in range [{:?}, {:?}): {:?}",
            write_start_offset,
            write_start_offset + write_len,
            write_result.unwrap_err()
        );
        info!("    write succeeded");
        if write_start_offset + write_len > self.contents.len() {
            self.contents.resize(write_start_offset + write_len, 0);
        }
        self.valid_len = self.valid_len.max(write_start_offset + write_len);
        self.contents[write_start_offset..write_start_offset + write_len]
            .copy_from_slice(&buf[..write_len]);
    }

    fn test_resize(&mut self, new_size: usize) {
        info!("Resize: name = {:?}, new_size = {:?}", self.name, new_size);
        // Todo: may need more consideration
        let resize_result = self.inode.resize(new_size);
        assert!(
            resize_result.is_ok(),
            "Fail to resize file to {:?}: {:?}",
            new_size,
            resize_result.unwrap_err()
        );
        self.contents.resize(new_size, 0);
        self.valid_len = self.valid_len.min(new_size);
    }

    pub fn execute_and_test(&mut self, op: Operation, rng: &mut dyn RngCore) {
        match op {
            Operation::Read(offset, len) => self.test_read(offset, len),
            Operation::Write(offset, len) => self.test_write(offset, len, rng),
            Operation::Resize(new_size) => self.test_resize(new_size),
            _ => {}
        }
    }
}

impl DentryInMemory {
    pub fn execute_and_test(&mut self, op: Operation, rng: &mut dyn RngCore) {
        match self {
            DentryInMemory::Dir(dir) => {
                dir.execute_and_test(op);
            }
            DentryInMemory::File(file) => {
                file.execute_and_test(op, rng);
            }
        }
    }

    pub fn sub_cnt(&self) -> usize {
        match self {
            DentryInMemory::Dir(dir) => dir.sub_names.len(),
            DentryInMemory::File(file) => 0,
        }
    }
}

fn random_select_from_dir_tree<'a>(
    root: &'a mut DentryInMemory,
    rng: &mut dyn RngCore,
) -> &'a mut DentryInMemory {
    let sub_cnt = root.sub_cnt();
    if sub_cnt == 0 {
        root
    } else {
        let stop_get_deeper = rng.gen_bool(0.5);
        if stop_get_deeper {
            root
        } else if let DentryInMemory::Dir(dir) = root {
            let sub_idx = rng.gen_range(0..sub_cnt);
            let sub = dir.sub_dirs.get_mut(&dir.sub_names[sub_idx]);
            let sub_dir = sub.unwrap();
            random_select_from_dir_tree(sub_dir, rng)
        } else {
            unreachable!();
        }
    }
}

fn generate_random_offset_len(max_size: usize, rng: &mut dyn RngCore) -> (usize, usize) {
    let offset = rng.gen_range(0..max_size);
    let len = rng.gen_range(0..max_size - offset);
    (offset, len)
}

pub fn new_fs_in_memory(root: Arc<dyn Inode>) -> DentryInMemory {
    DentryInMemory::Dir(DirInMemory {
        depth: 0,
        name: (&"root").to_string(),
        inode: root,
        sub_names: Vec::new(),
        sub_dirs: HashMap::new(),
    })
}
pub fn generate_random_operation<'a>(
    root: &'a mut DentryInMemory,
    idx: u32,
    rng: &mut dyn RngCore,
) -> (&'a mut DentryInMemory, Operation) {
    let dentry = random_select_from_dir_tree(root, rng);
    let op = match dentry {
        DentryInMemory::Dir(dir) => Operation::generate_random_dir_operation(dir, idx, rng),
        DentryInMemory::File(file) => Operation::generate_random_file_operation(file, idx, rng),
    };
    (dentry, op)
}
