// SPDX-License-Identifier: MPL-2.0

//! The synthetic uevent.
//!
//! The event is triggered when someone writes some content to `/sys/.../uevent` file.
//! It differs from the event triggers by devices.
//!
//! Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/Documentation/ABI/testing/sysfs-uevent>.

use alloc::format;
use core::str::FromStr;

use super::uevent::SysObjAction;
use crate::prelude::*;

/// The synthetic uevent.
pub(super) struct SyntheticUevent {
    pub(super) action: SysObjAction,
    pub(super) uuid: Option<Uuid>,
    pub(super) envs: Vec<(String, String)>,
}

impl FromStr for SyntheticUevent {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut split = s.split(" ");

        let action = {
            let Some(action_str) = split.next() else {
                return_errno_with_message!(Errno::EINVAL, "the string is empty");
            };
            SysObjAction::from_str(action_str)?
        };

        let uuid = if let Some(uuid_str) = split.next() {
            Some(Uuid::from_str(uuid_str)?)
        } else {
            None
        };

        let mut envs = Vec::new();
        for env_str in split {
            let (key, value) = {
                // Each string should be in the `KEY=VALUE` format.
                match env_str.split_once('=') {
                    Some(key_value) => key_value,
                    None => return_errno_with_message!(Errno::EINVAL, "invalid key value pairs"),
                }
            };

            // Both `KEY` and `VALUE` can contain alphanumeric characters only.
            for byte in key.as_bytes().iter().chain(value.as_bytes()) {
                if !byte.is_ascii_alphanumeric() {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "invalid character in key value pairs"
                    );
                }
            }

            // The `KEY` name gains `SYNTH_ARG_` prefix to avoid possible collisions
            // with existing variables.
            let key = format!("SYNTH_ARG_{}", key);
            let value = value.to_string();
            envs.push((key, value));
        }

        Ok(Self { action, uuid, envs })
    }
}

pub(super) struct Uuid(pub(super) String);

impl FromStr for Uuid {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        /// The allowed UUID pattern, where each `x` is a hex digit.
        ///
        /// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/Documentation/ABI/testing/sysfs-uevent#L19>.
        const UUID_PATTERN: &str = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx";

        let bytes = s.as_bytes();

        if bytes.len() != UUID_PATTERN.len() {
            return_errno_with_message!(Errno::EINVAL, "the UUID length is invalid");
        }

        for (byte, pattern) in bytes.iter().zip(UUID_PATTERN.as_bytes()) {
            if (*pattern == b'x' && byte.is_ascii_hexdigit()) || (*pattern == b'-' && *byte == b'-')
            {
                continue;
            } else {
                return_errno_with_message!(Errno::EINVAL, "the UUID content is invalid");
            }
        }

        Ok(Self(s.to_string()))
    }
}
