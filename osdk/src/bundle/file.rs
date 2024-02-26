// SPDX-License-Identifier: MPL-2.0

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// A trait for files in a bundle. The file in a bundle should have it's digest and be validatable.
pub trait BundleFile {
    fn path(&self) -> &PathBuf;

    fn sha256sum(&self) -> &String;

    fn calculate_sha256sum(&self) -> String {
        let mut file = fs::File::open(self.path()).unwrap();
        let mut hasher = Sha256::new();
        let _n = io::copy(&mut file, &mut hasher).unwrap();
        format!("{:x}", hasher.finalize())
    }

    fn validate(&self) -> bool {
        self.sha256sum() == &self.calculate_sha256sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Initramfs {
    path: PathBuf,
    sha256sum: String,
}

impl BundleFile for Initramfs {
    fn path(&self) -> &PathBuf {
        &self.path
    }

    fn sha256sum(&self) -> &String {
        &self.sha256sum
    }
}

impl Initramfs {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let created = Self {
            path: path.as_ref().to_path_buf(),
            sha256sum: String::new(),
        };
        Self {
            sha256sum: created.calculate_sha256sum(),
            ..created
        }
    }

    /// Move the initramfs to the `base` directory and convert the path to a relative path.
    pub fn copy_to(self, base: impl AsRef<Path>) -> Self {
        let name = self.path.file_name().unwrap();
        let dest = base.as_ref().join(name);
        fs::copy(&self.path, dest).unwrap();
        Self {
            path: PathBuf::from(name),
            ..self
        }
    }
}
