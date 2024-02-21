// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterBin {
    pub path: PathBuf,
    pub typ: AsterBinType,
    pub version: String,
    pub sha256sum: String,
    pub stripped: bool,
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
