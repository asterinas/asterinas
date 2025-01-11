// SPDX-License-Identifier: MPL-2.0

use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::file::BundleFile;
use crate::{arch::Arch, util::hard_link_or_copy};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterBin {
    path: PathBuf,
    arch: Arch,
    typ: AsterBinType,
    version: String,
    modified_time: SystemTime,
    size: u64,
    stripped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsterBinType {
    Elf(AsterElfMeta),
    BzImage(AsterBzImageMeta),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterElfMeta {
    pub has_linux_header: bool,
    pub has_pvh_header: bool,
    pub has_multiboot_header: bool,
    pub has_multiboot2_header: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterBzImageMeta {
    pub support_legacy32_boot: bool,
    pub support_efi_boot: bool,
    pub support_efi_handover: bool,
}

impl BundleFile for AsterBin {
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

impl AsterBin {
    pub fn new(
        path: impl AsRef<Path>,
        arch: Arch,
        typ: AsterBinType,
        version: String,
        stripped: bool,
    ) -> Self {
        let created = Self {
            path: path.as_ref().to_path_buf(),
            arch,
            typ,
            version,
            modified_time: SystemTime::UNIX_EPOCH,
            size: 0,
            stripped,
        };
        Self {
            modified_time: created.get_modified_time(),
            size: created.get_size(),
            ..created
        }
    }

    pub fn arch(&self) -> Arch {
        self.arch
    }

    pub fn version(&self) -> &String {
        &self.version
    }

    pub fn stripped(&self) -> bool {
        self.stripped
    }

    /// Copy the binary to the `base` directory and convert the path to a relative path.
    pub fn copy_to(self, base: impl AsRef<Path>) -> Self {
        let file_name = self.path.file_name().unwrap();
        let copied_path = base.as_ref().join(file_name);
        hard_link_or_copy(&self.path, &copied_path).unwrap();
        let copied_metadata = copied_path.metadata().unwrap();
        Self {
            path: PathBuf::from(file_name),
            arch: self.arch,
            typ: self.typ,
            version: self.version,
            modified_time: copied_metadata.modified().unwrap(),
            size: copied_metadata.size(),
            stripped: self.stripped,
        }
    }
}
