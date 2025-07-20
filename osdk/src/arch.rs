// SPDX-License-Identifier: MPL-2.0

use clap::{builder::PossibleValue, ValueEnum};
use std::fmt::{self, Display, Formatter};

/// Supported architectures.
///
/// The target triple for each architecture is fixed and shall not
/// be assigned by the user. This is also different from the first
/// element of the target triple, but akin to the "target_arch" cfg
/// of Cargo:
/// <https://doc.rust-lang.org/reference/conditional-compilation.html#target_arch>
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Arch {
    #[serde(rename = "aarch64")]
    Aarch64,
    #[serde(rename = "riscv64")]
    RiscV64,
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "loongarch64")]
    LoongArch64,
}

impl ValueEnum for Arch {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Arch::Aarch64,
            Arch::RiscV64,
            Arch::X86_64,
            Arch::LoongArch64,
        ]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            Arch::Aarch64 => Some(PossibleValue::new(self.to_str())),
            Arch::RiscV64 => Some(PossibleValue::new(self.to_str())),
            Arch::X86_64 => Some(PossibleValue::new(self.to_str())),
            Arch::LoongArch64 => Some(PossibleValue::new(self.to_str())),
        }
    }
}

impl Arch {
    /// Get the target triple for the architecture.
    pub fn triple(&self) -> &'static str {
        match self {
            Arch::Aarch64 => "aarch64-unknown-none-softfloat",
            Arch::RiscV64 => "riscv64imac-unknown-none-elf",
            Arch::X86_64 => "x86_64-unknown-none",
            Arch::LoongArch64 => "loongarch64-unknown-none-softfloat",
        }
    }

    pub fn system_qemu(&self) -> &'static str {
        match self {
            Arch::Aarch64 => "qemu-system-aarch64",
            Arch::RiscV64 => "qemu-system-riscv64",
            Arch::X86_64 => "qemu-system-x86_64",
            Arch::LoongArch64 => "qemu-system-loongarch64",
        }
    }

    pub fn to_str(self) -> &'static str {
        match self {
            Arch::Aarch64 => "aarch64",
            Arch::RiscV64 => "riscv64",
            Arch::X86_64 => "x86_64",
            Arch::LoongArch64 => "loongarch64",
        }
    }
}

impl Display for Arch {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

/// Get the default architecture implied by the host rustc's default architecture.
pub fn get_default_arch() -> Arch {
    let output = crate::util::new_command_checked_exists("rustc")
        .arg("-vV")
        .output()
        .expect("Failed to run rustc to get the host target");
    let output =
        std::str::from_utf8(&output.stdout).expect("`rustc -vV` didn't return utf8 output");

    let field = "host: ";
    let host = output
        .lines()
        .find(|l| l.starts_with(field))
        .map(|l| &l[field.len()..])
        .expect("`rustc -vV` didn't give a line for host")
        .to_string();

    match host.split('-').next() {
        Some(host_arch) => match host_arch {
            "aarch64" => Arch::Aarch64,
            "riscv64gc" => Arch::RiscV64,
            "x86_64" => Arch::X86_64,
            "loongarch64" => Arch::LoongArch64,
            _ => panic!("The host has an unsupported native architecture"),
        },
        None => panic!("`rustc -vV` gave a host with unknown format"),
    }
}
