// SPDX-License-Identifier: MPL-2.0

//! A module about QEMU settings and arguments.

use std::{path::PathBuf, process};

use crate::{
    arch::{get_default_arch, Arch},
    config::unix_args::{apply_kv_array, get_key, split_to_kv_array},
    error::Errno,
    error_msg,
};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QemuScheme {
    /// The additional arguments for running QEMU, in the form of raw
    /// command line arguments.
    pub args: Option<String>,
    /// The additional qemu argument after the `-drive` option for the
    /// boot device, in the form of raw command line arguments, comma
    /// included. For example, `",if=virtio,media=disk,index=2"`.
    /// The `format` and the `file` options are not allowed to set.
    ///
    /// This option only works with [`super::BootMethod::GrubQcow2`] and
    /// [`super::BootMethod::GrubRescueIso`].
    ///
    /// This option exist because different firmwares may need
    /// different interface types for the boot drive.
    ///
    /// See <https://www.qemu.org/docs/master/system/invocation.html>
    /// for details about `-drive` option.
    pub bootdev_append_options: Option<String>,
    /// The path of qemu
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Qemu {
    pub args: String,
    /// This finalized config has a unorthodox `Option` because
    /// we cannot provide a default value for it. The default
    /// value is determined by the final running routine
    /// [`crate::bundle::Bundle::run`].
    pub bootdev_append_options: Option<String>,
    pub path: PathBuf,
}

impl Default for Qemu {
    fn default() -> Self {
        Qemu {
            args: String::new(),
            bootdev_append_options: None,
            path: PathBuf::from(get_default_arch().system_qemu()),
        }
    }
}

impl Qemu {
    pub fn apply_qemu_args(&mut self, args: &Vec<String>) {
        let mut joined = split_to_kv_array(&self.args);

        // Check the soundness of qemu arguments
        for arg in joined.iter() {
            check_qemu_arg(arg);
        }

        apply_kv_array(&mut joined, args, " ", MULTI_VALUE_KEYS);

        self.args = joined.join(" ");
    }
}

impl QemuScheme {
    pub fn inherit(&mut self, from: &Self) {
        if self.args.is_none() {
            self.args.clone_from(&from.args);
        }
        if self.path.is_none() {
            self.path.clone_from(&from.path);
        }
    }

    pub fn finalize(self, arch: Arch) -> Qemu {
        Qemu {
            args: self.args.unwrap_or_default(),
            bootdev_append_options: self.bootdev_append_options,
            path: self.path.unwrap_or(PathBuf::from(arch.system_qemu())),
        }
    }
}

// Below are checked keys in qemu arguments. The key list is non-exhaustive.

/// Keys with multiple values
const MULTI_VALUE_KEYS: &[&str] = &[
    "-device", "-chardev", "-object", "-netdev", "-drive", "-cdrom",
];
/// Keys with only single value
const SINGLE_VALUE_KEYS: &[&str] = &["-cpu", "-machine", "-m", "-serial", "-monitor", "-display"];
/// Keys with no value
const NO_VALUE_KEYS: &[&str] = &["--no-reboot", "-nographic", "-enable-kvm"];
/// Keys are not allowed to set in configuration files and command line
const NOT_ALLOWED_TO_SET_KEYS: &[&str] = &["-kernel", "-append", "-initrd"];

fn check_qemu_arg(arg: &str) {
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
