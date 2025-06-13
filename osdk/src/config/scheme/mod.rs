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
    /// The working directory.
    ///
    /// The user is not allowed to set this field. However,
    /// the manifest loader set this and all actions such
    /// as running, testing, and building will use this field.
    pub work_dir: Option<PathBuf>,
    #[serde(default)]
    pub supported_archs: Vec<Arch>,
    /// The boot configs.
    ///
    /// Building, running, and testing would consult on these configs.
    pub boot: Option<BootScheme>,
    /// The GRUB configs.
    ///
    /// Building, running, and testing would consult on these configs.
    pub grub: Option<GrubScheme>,
    /// The QEMU configs.
    ///
    /// Building, running, and testing would consult on these configs.
    pub qemu: Option<QemuScheme>,
    /// Other build configs.
    ///
    /// Building, running, and testing would consult on these configs.
    pub build: Option<BuildScheme>,
    /// Running specific configs.
    ///
    /// These values, if exists, overrides global boot/GRUB/QEMU/build configs.
    pub run: Option<ActionScheme>,
    /// Testing specific configs.
    ///
    /// These values, if exists, overrides global boot/GRUB/QEMU/build configs.
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

    /// Produces a scheme in which `run` and `test` action settings inherit
    /// from the default settings in the provided scheme.
    pub fn run_and_test_inherit_from_global(&mut self) {
        let old_scheme = self.clone();

        if let Some(run) = &mut self.run {
            inherit_optional!(old_scheme, run, .boot);
            inherit_optional!(old_scheme, run, .grub);
            inherit_optional!(old_scheme, run, .qemu);
            inherit_optional!(old_scheme, run, .build);
        } else {
            self.run = Some(ActionScheme {
                boot: self.boot.clone(),
                grub: self.grub.clone(),
                qemu: self.qemu.clone(),
                build: self.build.clone(),
            });
        }

        if let Some(test) = &mut self.test {
            inherit_optional!(old_scheme, test, .boot);
            inherit_optional!(old_scheme, test, .grub);
            inherit_optional!(old_scheme, test, .qemu);
            inherit_optional!(old_scheme, test, .build);
        } else {
            self.test = Some(ActionScheme {
                boot: self.boot.clone(),
                grub: self.grub.clone(),
                qemu: self.qemu.clone(),
                build: self.build.clone(),
            });
        }
    }
}
