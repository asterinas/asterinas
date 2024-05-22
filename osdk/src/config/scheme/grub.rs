// SPDX-License-Identifier: MPL-2.0

use clap::ValueEnum;

use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrubScheme {
    /// The path of `grub_mkrecue`. Only needed if `boot.method` is `grub`
    pub grub_mkrescue: Option<PathBuf>,
    /// The boot protocol specified in the GRUB configuration
    pub boot_protocol: Option<BootProtocol>,
    /// Whether to display the GRUB menu, defaults to `false`
    #[serde(default)]
    pub display_grub_menu: bool,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum BootProtocol {
    Linux,
    Multiboot,
    #[default]
    Multiboot2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grub {
    pub grub_mkrescue: PathBuf,
    pub boot_protocol: BootProtocol,
    pub display_grub_menu: bool,
}

impl Default for Grub {
    fn default() -> Self {
        Grub {
            grub_mkrescue: PathBuf::from("grub-mkrescue"),
            boot_protocol: BootProtocol::default(),
            display_grub_menu: false,
        }
    }
}

impl GrubScheme {
    pub fn inherit(&mut self, from: &Self) {
        if self.grub_mkrescue.is_none() {
            self.grub_mkrescue.clone_from(&from.grub_mkrescue);
        }
        if self.boot_protocol.is_none() {
            self.boot_protocol = from.boot_protocol;
        }
        // `display_grub_menu` is not inherited
    }

    pub fn finalize(self) -> Grub {
        Grub {
            grub_mkrescue: self.grub_mkrescue.unwrap_or(PathBuf::from("grub-mkrescue")),
            boot_protocol: self.boot_protocol.unwrap_or(BootProtocol::Multiboot2),
            display_grub_menu: self.display_grub_menu,
        }
    }
}
