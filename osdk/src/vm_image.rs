// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterVmImage {
    pub path: PathBuf,
    pub typ: AsterVmImageType,
    pub aster_version: String,
    pub sha256sum: String,
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
