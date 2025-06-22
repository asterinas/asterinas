// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Display,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
};

use super::syn_uevent::{SyntheticUevent, Uuid};
use crate::prelude::*;

/// `SysObj` action type.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.14/source/include/linux/kobject.h#L53>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[repr(u8)]
pub(super) enum SysObjAction {
    /// Indicates the addition of a new `SysObj` to the system.
    ///
    /// Triggered when a device is discovered or registered.
    Add = 0,

    /// Signals the removal of a `SysObj` from the system.
    ///
    /// Typically occurs during device disconnection or deregistration.
    Remove = 1,

    /// Denotes a modification to the `SysObj`'s properties or state.
    ///
    /// Used for attribute changes that don't involve structural modifications.
    Change = 2,

    /// Represents hierarchical relocation of a `SysObj`.
    ///
    /// Occurs when a device moves within the device tree topology.
    Move = 3,

    /// Marks a device returning to operational status after being offlined.
    ///
    /// Common in hot-pluggable device scenarios.
    Online = 4,

    /// Indicates a device entering non-operational status.
    ///
    /// Typically precedes safe removal of hot-pluggable hardware.
    Offline = 5,

    /// Signifies successful driver-device binding.
    ///
    /// Occurs after successful driver probe sequence.
    Bind = 6,

    /// Indicates driver-device binding termination.
    ///
    /// Precedes driver unload or device removal.
    Unbind = 7,
}

const SYSOBJ_ACTION_STRS: [&str; SysObjAction::Unbind as usize + 1] = [
    "add", "remove", "change", "move", "online", "offline", "bind", "unbind",
];

impl FromStr for SysObjAction {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let Some(index) = SYSOBJ_ACTION_STRS
            .iter()
            .position(|action_str| s == *action_str)
        else {
            return_errno_with_message!(Errno::EINVAL, "the string is not a valid `SysObj` action");
        };

        Ok(SysObjAction::try_from(index as u8).unwrap())
    }
}

impl SysObjAction {
    fn as_str(&self) -> &'static str {
        SYSOBJ_ACTION_STRS[*self as usize]
    }
}

/// Userspace event.
pub(super) struct Uevent {
    /// The `SysObj` action.
    action: SysObjAction,
    /// The absolute `SysObj` path under sysfs.
    devpath: String,
    /// The subsystem the event originates from
    subsystem: String,
    /// Other key-value arguments
    envs: Vec<(String, String)>,
    /// Sequence number.
    seq_num: u64,
}

impl Uevent {
    /// Creates a new uevent.
    fn new(
        action: SysObjAction,
        devpath: String,
        subsystem: String,
        envs: Vec<(String, String)>,
    ) -> Self {
        debug_assert!(devpath.starts_with('/'));

        let seq_num = SEQ_NUM_ALLOCATOR.fetch_add(1, Ordering::Relaxed);

        Self {
            action,
            devpath,
            subsystem,
            envs,
            seq_num,
        }
    }

    /// Creates a new uevent from synthetic uevent.
    pub(super) fn new_from_syn(
        synth_uevent: SyntheticUevent,
        devpath: String,
        subsystem: String,
        mut other_envs: Vec<(String, String)>,
    ) -> Self {
        let SyntheticUevent {
            action,
            uuid,
            mut envs,
        } = synth_uevent;

        let uuid_key = "SYNTH_UUID".to_string();
        if let Some(Uuid(uuid)) = uuid {
            envs.push((uuid_key, uuid));
        } else {
            envs.push((uuid_key, "0".to_string()));
        };

        envs.append(&mut other_envs);

        Self::new(action, devpath, subsystem, envs)
    }
}

impl Display for Uevent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut env_string = {
            let len = self
                .envs
                .iter()
                .map(|(key, value)| key.len() + value.len() + 2)
                .sum();
            String::with_capacity(len)
        };

        for (key, value) in self.envs.iter() {
            env_string.push_str(key);
            env_string.push('=');
            env_string.push_str(value);
            env_string.push('\0');
        }

        write!(
            f,
            "{}@{}\0ACTION={}\0DEVPATH={}\0SUBSYSTEM={}\0{}SEQNUM={}\0",
            self.action.as_str(),
            self.devpath,
            self.action.as_str(),
            self.devpath,
            self.subsystem,
            env_string,
            self.seq_num
        )
    }
}

static SEQ_NUM_ALLOCATOR: AtomicU64 = AtomicU64::new(1);
