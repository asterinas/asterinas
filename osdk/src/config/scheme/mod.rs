// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use crate::arch::Arch;

mod action;
pub use action::*;
mod boot;
pub use boot::*;
mod grub;
pub use grub::*;
mod qemu;
pub use qemu::*;

/// All the configurable fields within a scheme.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scheme {
    // The user is not allowed to set this field. However,
    // the manifest loader set this and all actions such
    // as running, testing, and building will use this field.
    pub work_dir: Option<PathBuf>,
    #[serde(default)]
    pub supported_archs: Vec<Arch>,
    pub boot: Option<BootScheme>,
    pub grub: Option<GrubScheme>,
    pub qemu: Option<QemuScheme>,
    pub build: Option<BuildScheme>,
    pub run: Option<ActionScheme>,
    pub test: Option<ActionScheme>,
}

macro_rules! inherit_optional {
    ($from:ident, $to:ident, .$field:ident) => {
        if $to.$field.is_none() {
            $to.$field = $from.$field.clone();
        } else {
            if let Some($field) = &$from.$field {
                $to.$field.as_mut().unwrap().inherit($field);
            }
        }
    };
}
use inherit_optional;

impl Scheme {
    pub fn empty() -> Self {
        Scheme {
            work_dir: None,
            supported_archs: vec![],
            boot: None,
            grub: None,
            qemu: None,
            build: None,
            run: None,
            test: None,
        }
    }

    pub fn inherit(&mut self, from: &Self) {
        // Supported archs are not inherited
        inherit_optional!(from, self, .boot);
        inherit_optional!(from, self, .grub);
        inherit_optional!(from, self, .build);
        inherit_optional!(from, self, .run);
        inherit_optional!(from, self, .test);
        // The inheritance of `work_dir` depends on `qemu`, so
        // here is a special treatment.
        if let Some(qemu) = &mut self.qemu {
            if let Some(from_qemu) = &from.qemu {
                if qemu.args.is_none() {
                    qemu.args.clone_from(&from_qemu.args);
                    self.work_dir.clone_from(&from.work_dir);
                }
                if qemu.path.is_none() {
                    qemu.path.clone_from(&from_qemu.path);
                    self.work_dir.clone_from(&from.work_dir);
                }
            }
        } else {
            self.qemu.clone_from(&from.qemu);
            self.work_dir.clone_from(&from.work_dir);
        }
    }
}
