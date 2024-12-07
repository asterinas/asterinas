// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// A trait for files in a bundle. The file in a bundle should have its modified time and be validatable.
pub trait BundleFile {
    fn path(&self) -> &PathBuf;

    fn modified_time(&self) -> &SystemTime;

    fn get_modified_time(&self) -> SystemTime {
        self.path().metadata().unwrap().modified().unwrap()
    }

    fn validate(&self) -> bool {
        self.modified_time() >= &self.get_modified_time()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Initramfs {
    path: PathBuf,
    modified_time: SystemTime,
}

impl BundleFile for Initramfs {
    fn path(&self) -> &PathBuf {
        &self.path
    }

    fn modified_time(&self) -> &SystemTime {
        &self.modified_time
    }
}

impl Initramfs {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let created = Self {
            path: path.as_ref().to_path_buf(),
            modified_time: SystemTime::UNIX_EPOCH,
        };
        Self {
            modified_time: created.get_modified_time(),
            ..created
        }
    }

    /// Copy the initramfs to the `base` directory and convert the path to a relative path.
    pub fn copy_to(self, base: impl AsRef<Path>) -> Self {
        let name = self.path.file_name().unwrap();
        let dest = base.as_ref().join(name);
        // Do not use hard_link/symlink here, otherwise `cargo build` will fail when initramfs is updated.
        info!("Copying {:?} -> {:?}", &self.path, &dest);
        fs::copy(&self.path, &dest).unwrap();
        Self {
            path: PathBuf::from(name),
            modified_time: dest.metadata().unwrap().modified().unwrap(),
        }
    }
}
