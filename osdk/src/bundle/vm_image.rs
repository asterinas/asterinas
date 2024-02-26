// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::file::BundleFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterVmImage {
    path: PathBuf,
    typ: AsterVmImageType,
    aster_version: String,
    sha256sum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsterVmImageType {
    GrubIso(AsterGrubIsoImageMeta),
    // TODO: add more vm image types such as qcow2, etc.
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterGrubIsoImageMeta {
    pub grub_version: String,
}

impl BundleFile for AsterVmImage {
    fn path(&self) -> &PathBuf {
        &self.path
    }

    fn sha256sum(&self) -> &String {
        &self.sha256sum
    }
}

impl AsterVmImage {
    pub fn new(path: impl AsRef<Path>, typ: AsterVmImageType, aster_version: String) -> Self {
        let created = Self {
            path: path.as_ref().to_path_buf(),
            typ,
            aster_version,
            sha256sum: String::new(),
        };
        Self {
            sha256sum: created.calculate_sha256sum(),
            ..created
        }
    }

    /// Move the binary to the `base` directory and convert the path to a relative path.
    pub fn move_to(self, base: impl AsRef<Path>) -> Self {
        let file_name = self.path.file_name().unwrap();
        let copied_path = base.as_ref().join(file_name);
        fs::copy(&self.path, copied_path).unwrap();
        fs::remove_file(&self.path).unwrap();
        Self {
            path: PathBuf::from(file_name),
            typ: self.typ,
            aster_version: self.aster_version,
            sha256sum: self.sha256sum,
        }
    }
}
