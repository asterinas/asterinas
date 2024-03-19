// SPDX-License-Identifier: MPL-2.0

use clap::{builder::PossibleValue, ValueEnum};
use std::fmt::{self, Display, Formatter};

/// A list of supported targets and the corresponding triple
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Target {
    X86_64,
    RiscV64,
}

impl ValueEnum for Target {
    fn value_variants<'a>() -> &'a [Self] {
        &[Target::X86_64, Target::RiscV64]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            Target::X86_64 => Some(PossibleValue::new(self.display_name())),
            Target::RiscV64 => Some(PossibleValue::new(self.display_name())),
        }
    }
}

impl Target {
    pub fn triple(&self) -> String {
        match self {
            Target::X86_64 => "x86_64-unknown-none".to_owned(),
            Target::RiscV64 => "riscv64gc-unknown-none-elf".to_owned(),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Target::X86_64 => "x86_64",
            Target::RiscV64 => "riscv64",
        }
    }
}

impl Display for Target {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Get the default target implied by the host architecture.
pub fn get_default_target() -> Target {
    let output = std::process::Command::new("rustc")
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
            "x86_64" => Target::X86_64,
            "riscv64gc" => Target::RiscV64,
            _ => panic!("The host has an unsupported native architecture"),
        },
        None => panic!("`rustc -vV` gave a host with unknown format"),
    }
}
