// SPDX-License-Identifier: MPL-2.0

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::util::fast_copy;

use super::file::BundleFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterVmImage {
    path: PathBuf,
    typ: AsterVmImageType,
    aster_version: String,
    modified_time: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsterVmImageType {
    GrubIso(AsterGrubIsoImageMeta),
    Qcow2(AsterQcow2ImageMeta),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterGrubIsoImageMeta {
    pub grub_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterQcow2ImageMeta {
    pub grub_version: String,
}

impl BundleFile for AsterVmImage {
    fn path(&self) -> &PathBuf {
        &self.path
    }

    fn modified_time(&self) -> &SystemTime {
        &self.modified_time
    }
}

impl AsterVmImage {
    pub fn new(path: impl AsRef<Path>, typ: AsterVmImageType, aster_version: String) -> Self {
        let created = Self {
            path: path.as_ref().to_path_buf(),
            typ,
            aster_version,
            modified_time: SystemTime::UNIX_EPOCH,
        };
        Self {
            modified_time: created.get_modified_time(),
            ..created
        }
    }

    pub fn typ(&self) -> &AsterVmImageType {
        &self.typ
    }

    /// Copy the binary to the `base` directory and convert the path to a relative path.
    pub fn copy_to(self, base: impl AsRef<Path>) -> Self {
        let file_name = self.path.file_name().unwrap();
        let copied_path = base.as_ref().join(file_name);
        fast_copy(&self.path, &copied_path).unwrap();
        Self {
            path: PathBuf::from(file_name),
            typ: self.typ,
            aster_version: self.aster_version,
            modified_time: copied_path.metadata().unwrap().modified().unwrap(),
        }
    }

    pub fn aster_version(&self) -> &String {
        &self.aster_version
    }
}
