// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::file::BundleFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterBin {
    path: PathBuf,
    typ: AsterBinType,
    version: String,
    sha256sum: String,
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

    fn sha256sum(&self) -> &String {
        &self.sha256sum
    }
}

impl AsterBin {
    pub fn new(path: impl AsRef<Path>, typ: AsterBinType, version: String, stripped: bool) -> Self {
        let created = Self {
            path: path.as_ref().to_path_buf(),
            typ,
            version,
            sha256sum: String::new(),
            stripped,
        };
        Self {
            sha256sum: created.calculate_sha256sum(),
            ..created
        }
    }

    pub fn version(&self) -> &String {
        &self.version
    }

    pub fn stripped(&self) -> bool {
        self.stripped
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
            version: self.version,
            sha256sum: self.sha256sum,
            stripped: self.stripped,
        }
    }
}
