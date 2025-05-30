// SPDX-License-Identifier: MPL-2.0

//! The synthetic uevent.
//!
//! The event is triggered when someone writes some content to `/sys/.../uevent` file.
//! It differs from the event triggers by devices.
//!
//! Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/Documentation/ABI/testing/sysfs-uevent>.

use alloc::format;
use core::str::FromStr;

use super::uevent::KobjectAction;
use crate::prelude::*;

/// The synthetic uevent.
pub struct SyntheticUevent {
    pub(super) action: KobjectAction,
    pub(super) uuid: Option<Uuid>,
    pub(super) envs: BTreeMap<String, String>,
}

impl FromStr for SyntheticUevent {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let split = s.split(" ").collect::<Vec<_>>();

        if split.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "the string is empty");
        }

        let action = KobjectAction::from_str(split[0])?;

        let uuid = if let Some(uuid_str) = split.get(1) {
            Some(Uuid::from_str(*uuid_str)?)
        } else {
            None
        };

        let mut envs = BTreeMap::new();
        for env_str in split.into_iter().skip(2) {
            // Each string should be in the `KEY=VALUE` format.
            let key_value = env_str.split('=').collect::<Vec<_>>();
            if key_value.len() != 2 {
                return_errno_with_message!(Errno::EINVAL, "invalid key value pairs");
            }

            // Both `KEY` and `VALUE` can contain alphanumeric characters only.
            for str in key_value.iter() {
                for byte in str.as_bytes() {
                    if !byte.is_ascii_alphanumeric() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "invalid character in key value pairs"
                        );
                    }
                }
            }

            // The KEY name gains ``SYNTH_ARG_`` prefix to avoid possible collisions
            // with existing variables.
            let key = format!("SYNTH_ARG_{}", key_value[0]);
            let value = key_value[1].to_string();
            envs.insert(key, value);
        }

        Ok(Self { action, uuid, envs })
    }
}

pub struct Uuid(pub(super) String);

impl FromStr for Uuid {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // The allowed UUID pattern is:
        // xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
        // where each `x` is a hex digit.
        // Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/Documentation/ABI/testing/sysfs-uevent#L19>.

        const UUID_LEN: usize = 36; // 8+1+4+1+4+1+4+1+12

        let bytes = s.as_bytes();

        if bytes.len() != UUID_LEN {
            return_errno_with_message!(Errno::EINVAL, "the uuid length is invalid");
        }

        const HYPHEN_INDEXES: [usize; 4] = [8, 13, 18, 23];

        for (index, byte) in bytes.into_iter().enumerate() {
            if HYPHEN_INDEXES.contains(&index) {
                if *byte == b'-' {
                    continue;
                }
                return_errno_with_message!(Errno::EINVAL, "the uuid content is invalid");
            }

            if !byte.is_ascii_hexdigit() {
                return_errno_with_message!(Errno::EINVAL, "the uuid content is invalid");
            }
        }

        Ok(Self(s.to_string()))
    }
}
