// SPDX-License-Identifier: MPL-2.0

use super::{inherit_optional, Boot, BootScheme, Grub, GrubScheme, Qemu, QemuScheme};

use crate::config::{scheme::Vars, Arch};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionChoice {
    Run,
    Test,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildScheme {
    pub profile: Option<String>,
    pub features: Vec<String>,
    /// Whether to turn on the support for the
    /// [Linux legacy x86 32-bit boot protocol](https://www.kernel.org/doc/html/v5.6/x86/boot.html)
    #[serde(default)]
    pub linux_x86_legacy_boot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    pub profile: String,
    pub features: Vec<String>,
    #[serde(default)]
    pub linux_x86_legacy_boot: bool,
}

impl Default for Build {
    fn default() -> Self {
        Self {
            profile: "dev".to_string(),
            features: Vec::new(),
            linux_x86_legacy_boot: false,
        }
    }
}

impl BuildScheme {
    pub fn inherit(&mut self, parent: &Self) {
        if parent.profile.is_some() {
            self.profile = parent.profile.clone();
        }
        self.features = {
            let mut features = parent.features.clone();
            features.extend(self.features.clone());
            features
        };
        if parent.linux_x86_legacy_boot {
            self.linux_x86_legacy_boot = true;
        }
    }

    pub fn finalize(self) -> Build {
        Build {
            profile: self.profile.unwrap_or_else(|| "dev".to_string()),
            features: self.features,
            linux_x86_legacy_boot: self.linux_x86_legacy_boot,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionScheme {
    #[serde(default)]
    pub vars: Vars,
    pub boot: Option<BootScheme>,
    pub grub: Option<GrubScheme>,
    pub qemu: Option<QemuScheme>,
    pub build: Option<BuildScheme>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub boot: Boot,
    pub grub: Grub,
    pub qemu: Qemu,
    pub build: Build,
}

impl ActionScheme {
    pub fn inherit(&mut self, from: &Self) {
        self.vars = {
            let mut vars = from.vars.clone();
            vars.extend(self.vars.clone());
            vars
        };
        inherit_optional!(from, self, .boot);
        inherit_optional!(from, self, .grub);
        inherit_optional!(from, self, .qemu);
        inherit_optional!(from, self, .build);
    }

    pub fn finalize(self, arch: Arch) -> Action {
        Action {
            boot: self.boot.unwrap_or_default().finalize(),
            grub: self.grub.unwrap_or_default().finalize(),
            qemu: self.qemu.unwrap_or_default().finalize(&self.vars, arch),
            build: self.build.unwrap_or_default().finalize(),
        }
    }
}
