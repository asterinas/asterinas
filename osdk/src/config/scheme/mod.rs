// SPDX-License-Identifier: MPL-2.0

use super::eval::Vars;

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
    #[serde(default)]
    pub supported_archs: Vec<Arch>,
    #[serde(default)]
    pub vars: Vars,
    pub boot: Option<BootScheme>,
    pub grub: Option<GrubScheme>,
    pub qemu: Option<QemuScheme>,
    pub build: Option<BuildScheme>,
    pub run: Option<ActionScheme>,
    pub test: Option<ActionScheme>,
}

macro_rules! inherit_optional {
    ($from: ident, $to:ident, .$field:ident) => {
        if $from.$field.is_some() {
            if let Some($field) = &mut $to.$field {
                $field.inherit($from.$field.as_ref().unwrap());
            } else {
                $to.$field = $from.$field.clone();
            }
        }
    };
}
use inherit_optional;

impl Scheme {
    pub fn empty() -> Self {
        Scheme {
            supported_archs: vec![],
            vars: vec![],
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

        self.vars = {
            let mut vars = from.vars.clone();
            vars.extend(self.vars.clone());
            vars
        };
        inherit_optional!(from, self, .boot);
        inherit_optional!(from, self, .grub);
        inherit_optional!(from, self, .qemu);
        inherit_optional!(from, self, .build);
        inherit_optional!(from, self, .run);
        inherit_optional!(from, self, .test);
    }
}
