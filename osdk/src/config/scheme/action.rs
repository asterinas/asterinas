// SPDX-License-Identifier: MPL-2.0

use linux_bzimage_builder::PayloadEncoding;

use super::{inherit_optional, Boot, BootScheme, Grub, GrubScheme, Qemu, QemuScheme};

use crate::{cli::CommonArgs, config::Arch};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionChoice {
    Run,
    Test,
    Miri,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildScheme {
    pub profile: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub no_default_features: bool,
    /// Whether to turn on the support for the
    /// [Linux legacy x86 32-bit boot protocol](https://www.kernel.org/doc/html/v5.6/x86/boot.html)
    #[serde(default)]
    pub linux_x86_legacy_boot: bool,
    #[serde(default)]
    pub strip_elf: bool,
    pub encoding: Option<PayloadEncoding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    pub profile: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub no_default_features: bool,
    // The cargo `--config` values.
    pub override_configs: Vec<String>,
    #[serde(default)]
    pub linux_x86_legacy_boot: bool,
    #[serde(default)]
    pub strip_elf: bool,
    pub encoding: PayloadEncoding,
}

impl Default for Build {
    fn default() -> Self {
        Self {
            profile: "dev".to_string(),
            features: Vec::new(),
            no_default_features: false,
            override_configs: Vec::new(),
            linux_x86_legacy_boot: false,
            strip_elf: false,
            encoding: PayloadEncoding::default(),
        }
    }
}

impl Build {
    pub fn apply_common_args(&mut self, common_args: &CommonArgs) {
        let build_args = &common_args.build_args;
        if let Some(profile) = build_args.profile() {
            self.profile.clone_from(&profile);
        }
        self.features.extend(build_args.features.clone());
        self.override_configs
            .extend(build_args.override_configs.clone());
        if common_args.build_args.no_default_features {
            self.no_default_features = true;
        }
        if common_args.linux_x86_legacy_boot {
            self.linux_x86_legacy_boot = true;
        }
        if common_args.strip_elf {
            self.strip_elf = true;
        }
        if let Some(encoding) = common_args.encoding.clone() {
            self.encoding.clone_from(&encoding);
        }
    }
}

impl BuildScheme {
    pub fn inherit(&mut self, parent: &Self) {
        if self.profile.is_none() {
            self.profile.clone_from(&parent.profile);
        }
        self.features = {
            let mut features = parent.features.clone();
            features.extend(self.features.clone());
            features
        };
        // `no_default_features` is not inherited
        if parent.linux_x86_legacy_boot {
            self.linux_x86_legacy_boot = true;
        }
        if parent.strip_elf {
            self.strip_elf = true;
        }
        if self.encoding.is_none() {
            self.encoding.clone_from(&parent.encoding);
        }
    }

    pub fn finalize(self) -> Build {
        Build {
            profile: self.profile.unwrap_or_else(|| "dev".to_string()),
            features: self.features,
            no_default_features: self.no_default_features,
            override_configs: Vec::new(),
            linux_x86_legacy_boot: self.linux_x86_legacy_boot,
            strip_elf: self.strip_elf,
            encoding: self.encoding.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionScheme {
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
        inherit_optional!(from, self, .boot);
        inherit_optional!(from, self, .grub);
        inherit_optional!(from, self, .qemu);
        inherit_optional!(from, self, .build);
    }

    pub fn finalize(self, arch: Arch) -> Action {
        Action {
            boot: self.boot.unwrap_or_default().finalize(),
            grub: self.grub.unwrap_or_default().finalize(),
            qemu: self.qemu.unwrap_or_default().finalize(arch),
            build: self.build.unwrap_or_default().finalize(),
        }
    }
}
