// SPDX-License-Identifier: MPL-2.0

//! Device uevents for `NETLINK_KOBJECT_UEVENT` (protocol 15, group 1).
//!
//! Events announce device registration, removal, or driver binding changes,
//! or carry explicitly requested notifications.
//! Each datagram starts with `action@devpath\0`,
//! followed by NUL-separated `KEY=VALUE` entries and no netlink header.
//!
//! Use [`emit_uevent`] to send an explicit notification for a device.
//!
//! Reading a device's `/sys/.../uevent` attribute returns its environment as
//! newline-separated `KEY=VALUE` entries, without `ACTION`, `DEVPATH`,
//! `SUBSYSTEM`, or `SEQNUM`. It neither broadcasts an event nor allocates
//! a sequence number.
//! Writing the attribute accepts `<action> [<UUID> [<KEY=VALUE> ...]]` and
//! broadcasts a synthetic event without changing the device's registration
//! or driver binding. The optional UUID becomes `SYNTH_UUID` (default `0`),
//! and arguments become `SYNTH_ARG_KEY=VALUE` entries in input order.
//!
//! [`current_seqnum`] backs the read-only `/sys/kernel/uevent_seqnum`.

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::common::{AnyDevice, Error, Result, node::SysTreeEdit};

/// Maximum number of environment entries.
const MAX_UEVENT_VARS: usize = 64;
/// Maximum bytes in the encoded `KEY=VALUE\0` environment;
/// the `action@devpath\0` prefix is not counted.
const MAX_UEVENT_BYTES: usize = 2048;
/// The most recently allocated sequence number.
static CURRENT_SEQNUM: AtomicU64 = AtomicU64::new(0);

/// Sends an explicit uevent for `dev` without changing device registration or binding.
///
/// Devices without a bus or class subsystem produce no event.
/// Returns [`Error::NotAdded`] if the device is not registered or has been removed.
/// Environment and broadcast-hook errors propagate.
/// Delivery is best effort: events are not queued or replayed,
/// and absent listeners or full receive queues lose the notification.
pub fn emit_uevent<D: AnyDevice + ?Sized>(dev: &Arc<D>, action: UeventAction) -> Result<()> {
    if !dev.base().is_added() {
        return Err(Error::NotAdded);
    }
    emit(dev.to_arc().as_ref(), action, &UeventVars::new())
}

/// Sends an event with `extra` entries appended after the device variables.
///
/// Lifecycle callers may use this while removing a device, before its
/// directory and node metadata are detached. Explicit callers must first
/// check that the device is registered.
pub(crate) fn emit(dev: &dyn AnyDevice, action: UeventAction, extra: &UeventVars) -> Result<()> {
    let subsystem = dev.subsystem();
    let Some(subsystem_name) = subsystem.name() else {
        return Ok(());
    };
    let devpath = dev.base().tree_path();
    let mut vars = build_vars(dev)?;
    for (key, value) in extra.iter() {
        vars.add(key, value)?;
    }

    UeventBuilder::new(action, devpath, subsystem_name.to_string(), vars)?
        .build()?
        .commit()
}

/// Returns the most recently allocated sequence number.
///
/// Reading does not allocate a number.
/// Numbers can have gaps and do not guarantee delivery of earlier events.
pub fn current_seqnum() -> u64 {
    CURRENT_SEQNUM.load(Ordering::Relaxed)
}

/// The device event being reported.
///
/// The device model emits [`Add`], [`Remove`], [`Bind`], and [`Unbind`]
/// automatically; the other actions are sent only when explicitly requested.
///
/// [`Add`]: UeventAction::Add
/// [`Remove`]: UeventAction::Remove
/// [`Bind`]: UeventAction::Bind
/// [`Unbind`]: UeventAction::Unbind
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UeventAction {
    Add,
    Remove,
    Change,
    Move,
    Online,
    Offline,
    Bind,
    Unbind,
}

impl UeventAction {
    /// Returns the lowercase name used in the datagram and in sysfs requests.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Change => "change",
            Self::Move => "move",
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Bind => "bind",
            Self::Unbind => "unbind",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "add" => Self::Add,
            "remove" => Self::Remove,
            "change" => Self::Change,
            "move" => Self::Move,
            "online" => Self::Online,
            "offline" => Self::Offline,
            "bind" => Self::Bind,
            "unbind" => Self::Unbind,
            _ => return None,
        })
    }
}

impl fmt::Display for UeventAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The ordered `KEY=VALUE` environment of a uevent.
///
/// Preserves append order and duplicate keys.
/// Each entry costs `key.len() + value.len() + 2` bytes, including `=` and NUL.
/// Limited to 64 entries and 2048 bytes, matching
/// [Linux's uevent limits](https://github.com/torvalds/linux/blob/v6.14/include/linux/kobject.h#L32-L34).
/// Complete events reserve four entries for `ACTION`, `DEVPATH`, `SUBSYSTEM`, and `SEQNUM`,
/// leaving at most 60 additional entries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UeventVars {
    entries: Vec<(String, String)>,
    total_bytes: usize,
}

impl UeventVars {
    /// Creates an empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of entries.
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the encoded size of all entries in bytes.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Iterates over the entries in append order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// Appends a `KEY=VALUE` entry.
    ///
    /// Returns [`Error::InvalidValue`] if the key is empty,
    /// contains `=` or NUL,
    /// or the formatted value contains NUL.
    /// Returns [`Error::MemoryBudgetExhausted`] if adding the entry
    /// would exceed the entry count or byte limit.
    /// The environment is unchanged on failure.
    pub fn add(&mut self, key: &str, value: impl fmt::Display) -> Result<()> {
        if key.is_empty() || key.contains('=') || key.contains('\0') {
            return Err(Error::InvalidValue);
        }
        debug_assert!(self.total_bytes <= MAX_UEVENT_BYTES);
        let key_bytes = key
            .len()
            .checked_add(b"=\0".len())
            .ok_or(Error::MemoryBudgetExhausted)?;
        let limit = (MAX_UEVENT_BYTES - self.total_bytes)
            .checked_sub(key_bytes)
            .ok_or(Error::MemoryBudgetExhausted)?;
        if self.len() == MAX_UEVENT_VARS {
            return Err(Error::MemoryBudgetExhausted);
        }
        // Bound formatting itself: a large value must not allocate an
        // unbounded temporary before the environment limit is checked.
        let mut writer = ValueWriter::new(limit);
        let result = write!(writer, "{value}");
        if let Some(error) = writer.error {
            return Err(error);
        }
        result?;
        self.total_bytes += key_bytes + writer.value.len();
        self.entries.push((String::from(key), writer.value));
        Ok(())
    }

    /// Moves entries from another environment into this one in order.
    ///
    /// Returns [`Error::MemoryBudgetExhausted`] if the combined entry count
    /// or byte size exceeds the limit, leaving this environment unchanged.
    fn append(&mut self, mut other: Self) -> Result<()> {
        if other.len() > MAX_UEVENT_VARS - self.len()
            || other.total_bytes > MAX_UEVENT_BYTES - self.total_bytes
        {
            return Err(Error::MemoryBudgetExhausted);
        }
        self.total_bytes += other.total_bytes;
        self.entries.append(&mut other.entries);
        Ok(())
    }
}

struct ValueWriter {
    value: String,
    limit: usize,
    error: Option<Error>,
}

impl ValueWriter {
    fn new(limit: usize) -> Self {
        Self {
            value: String::new(),
            limit,
            error: None,
        }
    }
}

impl Write for ValueWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.error.is_some() {
            return Err(fmt::Error);
        }
        if text.contains('\0') {
            self.error = Some(Error::InvalidValue);
            return Err(fmt::Error);
        }
        debug_assert!(self.value.len() <= self.limit);
        if text.len() > self.limit - self.value.len() {
            self.error = Some(Error::MemoryBudgetExhausted);
            return Err(fmt::Error);
        }
        self.value.push_str(text);
        Ok(())
    }
}

/// Validated event inputs awaiting capacity checks and a sequence number.
pub(crate) struct UeventBuilder {
    action: UeventAction,
    devpath: String,
    subsystem: String,
    vars: UeventVars,
}

impl UeventBuilder {
    /// Validates event inputs without allocating a sequence number.
    ///
    /// Returns [`Error::InvalidValue`] if the path is relative, names `/sys`,
    /// or contains NUL, or if the subsystem is empty or contains NUL.
    pub(crate) fn new(
        action: UeventAction,
        devpath: String,
        subsystem: String,
        vars: UeventVars,
    ) -> Result<Self> {
        if !devpath.starts_with('/')
            || devpath == "/sys"
            || devpath.starts_with("/sys/")
            || devpath.contains('\0')
            || subsystem.is_empty()
            || subsystem.contains('\0')
        {
            return Err(Error::InvalidValue);
        }
        Ok(Self {
            action,
            devpath,
            subsystem,
            vars,
        })
    }

    /// Builds and serializes the event, allocating its sequence number.
    ///
    /// Checks preceding fields before allocating `SEQNUM`;
    /// failure to append `SEQNUM` can still leave a sequence gap.
    /// Returns [`Error::MemoryBudgetExhausted`] if the environment exceeds its limits.
    pub(crate) fn build(self) -> Result<Uevent> {
        let mut vars = UeventVars::new();
        vars.add("ACTION", self.action.as_str())?;
        vars.add("DEVPATH", &self.devpath)?;
        vars.add("SUBSYSTEM", &self.subsystem)?;
        vars.append(self.vars)?;
        // Reserve the final entry before allocating a sequence number.
        if vars.len() == MAX_UEVENT_VARS {
            return Err(Error::MemoryBudgetExhausted);
        }
        let seqnum = CURRENT_SEQNUM
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        vars.add("SEQNUM", seqnum)?;
        let action = self.action.as_str();
        let mut payload =
            Vec::with_capacity(action.len() + self.devpath.len() + 2 + vars.total_bytes());
        payload.extend_from_slice(action.as_bytes());
        payload.push(b'@');
        payload.extend_from_slice(self.devpath.as_bytes());
        payload.push(0);
        for (key, value) in vars.iter() {
            payload.extend_from_slice(key.as_bytes());
            payload.push(b'=');
            payload.extend_from_slice(value.as_bytes());
            payload.push(0);
        }
        Ok(Uevent { payload })
    }
}

/// A serialized device event ready for broadcasting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Uevent {
    payload: Vec<u8>,
}

impl Uevent {
    /// Broadcasts the event, transferring ownership of its serialized payload.
    pub(crate) fn commit(self) -> Result<()> {
        crate::hooks::broadcast_uevent(self.payload)
    }
}

/// Parses a request written to a device's sysfs `uevent` attribute.
///
/// The accepted syntax is `<action> [<UUID> [<KEY=VALUE> ...]]`.
/// Exactly one ASCII space separates items;
/// leading, trailing, or repeated spaces are rejected.
/// One final `\n` or NUL is allowed.
/// The optional UUID has the 36-character `8-4-4-4-12` form
/// and is preserved verbatim as `SYNTH_UUID`;
/// when it is absent, `SYNTH_UUID=0` is used.
/// Each argument must be an ASCII alphanumeric `KEY=VALUE` pair,
/// and becomes `SYNTH_ARG_KEY=VALUE` in input order.
/// Duplicate keys are kept.
///
/// Malformed input and synthetic-environment overflow both return
/// [`Error::InvalidValue`], which user space observes as `EINVAL`.
pub(crate) fn parse_synthetic(input: &str) -> Result<(UeventAction, UeventVars)> {
    let request = input
        .strip_suffix('\n')
        .or_else(|| input.strip_suffix('\0'))
        .unwrap_or(input);
    if request.is_empty()
        || request
            .bytes()
            .any(|byte| matches!(byte, b'\0' | b'\n' | b'\r' | b'\t'))
    {
        return Err(Error::InvalidValue);
    }
    let mut parts = request.split(' ');
    let action = parts
        .next()
        .and_then(UeventAction::parse)
        .ok_or(Error::InvalidValue)?;
    let uuid = match parts.next() {
        Some(uuid) => {
            if !is_valid_uuid(uuid) {
                return Err(Error::InvalidValue);
            }
            uuid
        }
        None => "0",
    };
    let mut vars = UeventVars::new();
    vars.add("SYNTH_UUID", uuid)
        .map_err(|_| Error::InvalidValue)?;
    for token in parts {
        let (key, value) = token.split_once('=').ok_or(Error::InvalidValue)?;
        if key.is_empty()
            || value.is_empty()
            || !key.bytes().all(|byte| byte.is_ascii_alphanumeric())
            || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(Error::InvalidValue);
        }
        let synth_key = alloc::format!("SYNTH_ARG_{key}");
        vars.add(&synth_key, value)
            .map_err(|_| Error::InvalidValue)?;
    }
    Ok((action, vars))
}

/// Returns whether `uuid` has the 36-character `8-4-4-4-12` hexadecimal form.
fn is_valid_uuid(uuid: &str) -> bool {
    uuid.len() == 36
        && uuid.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

/// Writes the device's uevent environment as newline-separated `KEY=VALUE` entries.
///
/// Called for a sysfs `uevent` read.
/// Omits event-only fields and does not broadcast or allocate a sequence number.
/// Returns [`Error::NotAdded`] if the device has been removed.
pub(crate) fn show(dev: &dyn AnyDevice, writer: &mut dyn Write) -> Result<()> {
    let vars = build_vars(dev)?;
    if !dev.base().is_added() {
        return Err(Error::NotAdded);
    }
    for (key, value) in vars.iter() {
        writeln!(writer, "{key}={value}")?;
    }
    Ok(())
}

/// Parses and broadcasts a synthetic event for a sysfs `uevent` write.
///
/// Does not change device registration or binding.
/// [`parse_synthetic`] errors map to `EINVAL`;
/// capacity overflow after merging the device environment maps to `ENOMEM`.
pub(crate) fn store(dev: &dyn AnyDevice, text: &str) -> Result<()> {
    if !dev.base().is_added() {
        return Err(Error::NotAdded);
    }
    let (action, vars) = parse_synthetic(text)?;
    emit(dev, action, &vars)
}

/// Collects the device-owned `KEY=VALUE` entries.
///
/// Includes `MAJOR`, `MINOR`, and `DEVNAME` for a non-zero major number,
/// and `DEVMODE` for an explicit mode, including zero, as octal permission bits.
/// `DEVTYPE` and `DRIVER` reflect the current state.
/// Subsystem and device-type callbacks then append entries in that order.
pub(crate) fn build_vars(dev: &dyn AnyDevice) -> Result<UeventVars> {
    let mut vars = UeventVars::new();
    if let Some(devnum) = dev.devnum()
        && devnum.id().major().get() != 0
    {
        vars.add("MAJOR", devnum.id().major().get())?;
        vars.add("MINOR", devnum.id().minor().get())?;
        let mode = {
            let node = dev.base().devnode().lock();
            let request = node.as_ref().ok_or(Error::NotAdded)?;
            vars.add("DEVNAME", &request.path)?;
            request.mode.map(|mode| mode & 0o777)
        };
        if let Some(mode) = mode {
            if mode == 0 {
                vars.add("DEVMODE", "0")?;
            } else {
                vars.add("DEVMODE", format_args!("0{mode:o}"))?;
            }
        }
    }
    if let Some(name) = dev.type_name() {
        vars.add("DEVTYPE", name)?;
    }
    dev.append_uevent_vars(&mut vars)?;
    Ok(vars)
}
