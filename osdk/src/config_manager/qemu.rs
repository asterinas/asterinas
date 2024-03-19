// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::BTreeMap,
    fmt::{self, Display},
    path::PathBuf,
    process,
};

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
    pub cfg_map: Option<BTreeMap<Cfg, Qemu>>,
}

/// A configuration that looks like "cfg(k1=v1, k2=v2, ...)".
#[derive(Debug, Clone, Eq, Ord, PartialOrd, PartialEq, Serialize)]
pub struct Cfg(BTreeMap<String, String>);

impl Cfg {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn from_str(s: &str) -> Self {
        let s = s.trim();

        if !s.starts_with("cfg(") || !s.ends_with(')') {
            error_msg!("`{}` is not a valid configuration", s);
            process::exit(Errno::ParseMetadata as _);
        }
        let s = &s[4..s.len() - 1];

        let mut cfg = BTreeMap::new();
        for kv in s.split(',') {
            let kv: Vec<_> = kv.split('=').collect();
            if kv.len() != 2 {
                error_msg!("`{}` is not a valid configuration", s);
                process::exit(Errno::ParseMetadata as _);
            }
            cfg.insert(
                kv[0].trim().to_string(),
                kv[1].trim().trim_matches('\"').to_string(),
            );
        }
        Self(cfg)
    }

    pub fn check_allowed(&self, allowed_keys: &[&str]) -> bool {
        for (k, _) in self.0.iter() {
            if allowed_keys.iter().all(|&key| k != key) {
                return false;
            }
        }
        true
    }

    pub fn insert(&mut self, k: String, v: String) {
        self.0.insert(k, v);
    }
}

impl Display for Cfg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cfg(")?;
        for (i, (k, v)) in self.0.iter().enumerate() {
            write!(f, "{}={}", k, v)?;
            if i != self.0.len() - 1 {
                write!(f, ", ")?;
            }
        }
        write!(f, ")")
    }
}

impl CfgQemu {
    pub fn new(default: Qemu, cfg_map: Option<BTreeMap<Cfg, Qemu>>) -> Self {
        Self { default, cfg_map }
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
            Cfg(Cfg),
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
                            v => Ok(Field::Cfg(Cfg::from_str(v))),
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
                let mut cfg_map = BTreeMap::<Cfg, Qemu>::new();

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
                            cfg_map.insert(cfg, qemu_args);
                        }
                    }
                }

                Ok(CfgQemu::new(default, Some(cfg_map)))
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
    Virt,
}

impl<'a> From<&'a str> for QemuMachine {
    fn from(value: &'a str) -> Self {
        match value {
            "microvm" => Self::Microvm,
            "q35" => Self::Q35,
            "virt" => Self::Virt,
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
