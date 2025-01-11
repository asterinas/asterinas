// SPDX-License-Identifier: MPL-2.0

use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::util::hard_link_or_copy;

use super::file::BundleFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterVmImage {
    path: PathBuf,
    typ: AsterVmImageType,
    aster_version: String,
    modified_time: SystemTime,
    size: u64,
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

    fn size(&self) -> &u64 {
        &self.size
    }
}

impl AsterVmImage {
    pub fn new(path: impl AsRef<Path>, typ: AsterVmImageType, aster_version: String) -> Self {
        let created = Self {
            path: path.as_ref().to_path_buf(),
            typ,
            aster_version,
            modified_time: SystemTime::UNIX_EPOCH,
            size: 0,
        };
        Self {
            modified_time: created.get_modified_time(),
            size: created.get_size(),
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
        hard_link_or_copy(&self.path, &copied_path).unwrap();
        let copied_metadata = copied_path.metadata().unwrap();
        Self {
            path: PathBuf::from(file_name),
            typ: self.typ,
            aster_version: self.aster_version,
            modified_time: copied_metadata.modified().unwrap(),
            size: copied_metadata.size(),
        }
    }

    pub fn aster_version(&self) -> &String {
        &self.aster_version
    }
}
