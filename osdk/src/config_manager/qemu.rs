// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeMap, fmt, path::PathBuf, process};

use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer,
};

use super::get_key;
use crate::{error::Errno, error_msg};

/// Arguments for creating bootdev image and how to boot with vmm.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Qemu {
    /// The additional arguments for running qemu, except `-cpu` and `-machine`.
    #[serde(default)]
    pub args: Vec<String>,
    /// The additional drive files
    #[serde(default)]
    pub drive_files: Vec<DriveFile>,
    /// The `-machine` argument for running qemu.
    #[serde(default)]
    pub machine: QemuMachine,
    /// The path of qemu.
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveFile {
    #[serde(default)]
    pub path: PathBuf,
    #[serde(default)]
    pub append: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct CfgQemu {
    pub default: Qemu,
    pub cfg: Option<BTreeMap<String, Qemu>>,
}

impl CfgQemu {
    pub fn new(default: Qemu, cfg: Option<BTreeMap<String, Qemu>>) -> Self {
        Self { default, cfg }
    }
}

impl<'de> Deserialize<'de> for CfgQemu {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Path,
            Args,
            Machine,
            DriveFiles,
            Cfg(String),
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl<'de> Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("`path`, `args`, `machine`, `drive_files` or cfg")
                    }

                    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        match v {
                            "args" => Ok(Field::Args),
                            "machine" => Ok(Field::Machine),
                            "path" => Ok(Field::Path),
                            "drive_files" => Ok(Field::DriveFiles),
                            v => Ok(Field::Cfg(v.to_string())),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct CfgQemuVisitor;

        impl<'de> Visitor<'de> for CfgQemuVisitor {
            type Value = CfgQemu;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct CfgQemu")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut default = Qemu::default();
                let mut cfgs = BTreeMap::new();

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Args => {
                            default.args = map.next_value()?;
                        }
                        Field::Machine => {
                            default.machine = map.next_value()?;
                        }
                        Field::Path => {
                            default.path = map.next_value()?;
                        }
                        Field::DriveFiles => {
                            default.drive_files = map.next_value()?;
                        }
                        Field::Cfg(cfg) => {
                            let qemu_args = map.next_value()?;
                            cfgs.insert(cfg, qemu_args);
                        }
                    }
                }

                Ok(CfgQemu::new(default, Some(cfgs)))
            }
        }

        deserializer.deserialize_struct("CfgQemu", &["default", "cfg"], CfgQemuVisitor)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QemuMachine {
    Microvm,
    #[default]
    Q35,
}

impl<'a> From<&'a str> for QemuMachine {
    fn from(value: &'a str) -> Self {
        match value {
            "microvm" => Self::Microvm,
            "q35" => Self::Q35,
            _ => {
                error_msg!("{} is not a valid option for `qemu.machine`", value);
                process::exit(Errno::ParseMetadata as _);
            }
        }
    }
}

// Below are keys in qemu arguments. The key list is not complete.

/// Keys with multiple values
pub const MULTI_VALUE_KEYS: &[&str] = &["-device", "-chardev", "-object", "-netdev", "-drive"];
/// Keys with only single value
pub const SINGLE_VALUE_KEYS: &[&str] = &["-m", "-serial", "-monitor", "-display"];
/// Keys with no value
pub const NO_VALUE_KEYS: &[&str] = &["--no-reboot", "-nographic", "-enable-kvm"];
/// Keys are not allowed to set in configuration files and command line
pub const NOT_ALLOWED_TO_SET_KEYS: &[&str] = &["-cpu", "-machine", "-kernel", "-initrd", "-cdrom"];

pub fn check_qemu_arg(arg: &str) {
    let key = if let Some(key) = get_key(arg, " ") {
        key
    } else {
        arg.to_string()
    };

    if NOT_ALLOWED_TO_SET_KEYS.contains(&key.as_str()) {
        error_msg!("`{}` is not allowed to set", arg);
        process::exit(Errno::ParseMetadata as _);
    }

    if NO_VALUE_KEYS.contains(&key.as_str()) && key.as_str() != arg {
        error_msg!("`{}` cannot have value", arg);
        process::exit(Errno::ParseMetadata as _);
    }

    if (SINGLE_VALUE_KEYS.contains(&key.as_str()) || MULTI_VALUE_KEYS.contains(&key.as_str()))
        && key.as_str() == arg
    {
        error_msg!("`{}` should have value", arg);
        process::exit(Errno::ParseMetadata as _);
    }
}
