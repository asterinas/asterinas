// SPDX-License-Identifier: MPL-2.0

//! Uevents: the messages the kernel sends to user space when a device is
//! added, removed, or changes.
//!
//! The kernel crate's netlink code has a `Uevent` of its own, for the messages
//! user space sends and receives. The two should become one once delivery is
//! wired up, with the netlink side building its wire form from this type: a
//! component cannot depend on the kernel crate, so the surviving definition
//! has to be this one.

use alloc::{string::String, vec::Vec};
use core::{
    fmt::{self, Display, Write},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{Error, Result};

/// What happened to a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UeventAction {
    Add,
    Remove,
    Change,
    Bind,
    Unbind,
}

impl Display for UeventAction {
    /// The string user space sees in `ACTION=`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UeventAction::Add => "add",
            UeventAction::Remove => "remove",
            UeventAction::Change => "change",
            UeventAction::Bind => "bind",
            UeventAction::Unbind => "unbind",
        })
    }
}

impl core::str::FromStr for UeventAction {
    type Err = Error;

    /// Parses the string user space writes to a `uevent` file.
    fn from_str(s: &str) -> Result<Self> {
        match s.trim() {
            "add" => Ok(UeventAction::Add),
            "remove" => Ok(UeventAction::Remove),
            "change" => Ok(UeventAction::Change),
            "bind" => Ok(UeventAction::Bind),
            "unbind" => Ok(UeventAction::Unbind),
            _ => Err(Error::InvalidValue),
        }
    }
}

/// The `KEY=VALUE` variables of a uevent, in insertion order.
///
/// Order matters: it is the order user space reads them in, both in the
/// `uevent` file and on the netlink socket, and Linux's is reproduced here.
///
/// Linux calls this a `kobj_uevent_env`, because the first hotplug mechanism
/// ran a helper program with these as its environment variables. Nothing here
/// executes anything, so the name would only mislead.
#[derive(Clone, Debug, Default)]
pub struct UeventVars {
    vars: Vec<(String, String)>,
}

impl UeventVars {
    /// Creates an empty set of variables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a variable.
    pub fn add(&mut self, key: &str, value: impl Display) {
        let mut v = String::new();
        // Formatting into a `String` cannot fail.
        let _ = write!(v, "{}", value);
        self.vars.push((String::from(key), v));
    }

    /// Returns the variables.
    pub fn vars(&self) -> &[(String, String)] {
        &self.vars
    }

    /// Looks up a variable by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Writes the variables as `KEY=VALUE` lines, one per line.
    pub fn write_lines(&self, w: &mut dyn Write) -> fmt::Result {
        for (k, v) in &self.vars {
            writeln!(w, "{}={}", k, v)?;
        }
        Ok(())
    }
}

/// A complete uevent, ready to be broadcast.
///
/// The fields are private so that the sequence number can only come from the
/// counter in [`Self::new`].
#[derive(Clone, Debug)]
pub struct Uevent {
    action: UeventAction,
    devpath: String,
    subsystem: String,
    vars: UeventVars,
    seqnum: u64,
}

impl Uevent {
    /// Creates an event and assigns it the next sequence number.
    pub fn new(action: UeventAction, devpath: String, subsystem: String, vars: UeventVars) -> Self {
        static SEQNUM: AtomicU64 = AtomicU64::new(1);
        let seqnum = SEQNUM.fetch_add(1, Ordering::Relaxed);
        Self {
            action,
            devpath,
            subsystem,
            vars,
            seqnum,
        }
    }

    /// Returns what happened.
    pub fn action(&self) -> UeventAction {
        self.action
    }

    /// Returns the path of the device below the sysfs mount point, e.g.
    /// `/devices/virtual/mem/null`.
    pub fn devpath(&self) -> &str {
        &self.devpath
    }

    /// Returns the bus or class name.
    pub fn subsystem(&self) -> &str {
        &self.subsystem
    }

    /// Returns the device-specific variables (`MAJOR`, `DEVNAME`, ...).
    pub fn vars(&self) -> &UeventVars {
        &self.vars
    }

    /// Returns the system-wide sequence number, which increases with every
    /// event.
    pub fn seqnum(&self) -> u64 {
        self.seqnum
    }
}
