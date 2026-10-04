// SPDX-License-Identifier: MPL-2.0

//! Device uevent actions, environments, validation, and sequence numbers.
//!
//! A uevent announces an automatic device-model change (`add` on device
//! registration, `remove` on unregistration, or `bind`/`unbind` on driver
//! binding), or a notification requested by user space, to listeners on
//! `NETLINK_KOBJECT_UEVENT` (protocol 15, group 1). Each datagram starts
//! with `action@devpath\0`, followed by NUL-separated `KEY=VALUE` entries
//! and no netlink header.
//!
//! This module defines the eight ABI actions, the ordered `KEY=VALUE`
//! environment ([`UeventVars`]), the validated event ([`Uevent`]), and the
//! parser for requests written to a device's sysfs `uevent` attribute.
//!
//! Use [`emit_uevent`] to send an explicit notification for a device.
//! Writing `/sys/.../uevent` accepts `<action> [<UUID> [<KEY=VALUE> ...]]`
//! and sends the same kind of event through `parse_synthetic`.
//!
//! The environment is limited to 64 entries and 2048 bytes, matching Linux.
//! Automatic events are best effort; explicit events report environment and
//! hook errors to the caller.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::common::{self, AnyDevice, Error, Result};

/// Maximum number of environment entries, matching Linux's `UEVENT_NUM_ENVP`.
const MAX_UEVENT_VARS: usize = 64;
/// Maximum bytes in the encoded `KEY=VALUE\0` environment, matching Linux's
/// `UEVENT_BUFFER_SIZE`; the `action@devpath\0` prefix is not counted.
const MAX_UEVENT_BYTES: usize = 2048;
/// The most recently allocated sequence number.
static CURRENT_SEQNUM: AtomicU64 = AtomicU64::new(0);

/// Sends an explicit uevent for `dev`.
///
/// The action describes why the notification is sent; it does not ask the
/// device model to perform the operation. For example, `remove` or `bind`
/// only broadcasts an event, while the caller remains responsible for
/// unregistering the device or changing its driver.
///
/// Any of the eight actions may be passed. Automatic `add`, `remove`,
/// `bind`, and `unbind` events are sent by the device model itself. Devices
/// without a bus or class subsystem produce no event.
///
/// Errors from building the uevent environment or from the broadcast hook propagate.
/// A successful return does not guarantee delivery: events are not queued or
/// replayed, and listeners that are absent or whose receive queues are full
/// miss the notification.
pub fn emit_uevent<D: AnyDevice + ?Sized>(dev: &Arc<D>, action: UeventAction) -> Result<()> {
    common::uevent::emit(dev.to_arc().as_ref(), action, &UeventVars::new())
}

/// Returns the most recently allocated sequence number.
///
/// Reading does not allocate a number. Numbers can have gaps: an event that
/// fails after allocating its number consumes it without being sent. A
/// larger value therefore does not prove that every earlier event was
/// delivered.
pub fn current_seqnum() -> u64 {
    CURRENT_SEQNUM.load(Ordering::Relaxed)
}

/// The reason a uevent was sent.
///
/// These eight names are part of the kernel ABI. They do not require the
/// receiver to perform the same operation. The device model emits [`Add`],
/// [`Remove`], [`Bind`], and [`Unbind`] automatically; the other actions are
/// sent only when explicitly requested.
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
/// Entries keep their append order and duplicate keys are preserved. Each
/// entry costs `key.len() + value.len() + 2` encoded bytes (the `=` and the
/// terminating NUL). The container accepts up to 64 entries and 2048 bytes;
/// a complete event reserves four more entries for `ACTION`, `DEVPATH`,
/// `SUBSYSTEM`, and `SEQNUM`, so at most 60 additional entries fit.
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
    /// The key must be non-empty and must not contain `=` or NUL. The
    /// formatted value must not contain NUL. Both must fit the remaining
    /// entry and byte budget.
    ///
    /// Returns [`Error::InvalidValue`] or [`Error::NoMemory`] without
    /// modifying the environment when an input is invalid or does not fit.
    pub fn add(&mut self, key: &str, value: impl fmt::Display) -> Result<()> {
        if key.is_empty() || key.contains('=') || key.contains('\0') {
            return Err(Error::InvalidValue);
        }
        let key_bytes = key.len().checked_add(2).ok_or(Error::NoMemory)?;
        let limit = MAX_UEVENT_BYTES
            .checked_sub(self.total_bytes)
            .and_then(|remaining| remaining.checked_sub(key_bytes))
            .ok_or(Error::NoMemory)?;
        if self.len() == MAX_UEVENT_VARS {
            return Err(Error::NoMemory);
        }
        // Bound formatting itself: a large value must not allocate an
        // unbounded temporary before the environment limit is checked.
        let mut writer = ValueWriter {
            value: String::new(),
            limit,
            error: None,
        };
        let result = write!(writer, "{value}");
        if let Some(error) = writer.error {
            return Err(error);
        }
        result?;
        self.total_bytes += key_bytes + writer.value.len();
        self.entries.push((String::from(key), writer.value));
        Ok(())
    }

    /// Removes every entry with `key`, including duplicates.
    pub(crate) fn remove(&mut self, key: &str) {
        self.entries.retain(|(entry_key, value)| {
            if entry_key == key {
                self.total_bytes -= entry_key.len() + value.len() + 2;
                false
            } else {
                true
            }
        });
    }
}

struct ValueWriter {
    value: String,
    limit: usize,
    error: Option<Error>,
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
        if text.len() > self.limit - self.value.len() {
            self.error = Some(Error::NoMemory);
            return Err(fmt::Error);
        }
        self.value.push_str(text);
        Ok(())
    }
}

/// A complete environment that has not been validated or given a number.
///
/// Removal prepares the environment while the `/dev` node, links, and
/// attributes still exist, then commits it after they are gone. Keeping the
/// two steps separate also lets automatic events fail without affecting the
/// device operation.
pub(crate) struct PreparedUevent {
    action: UeventAction,
    devpath: String,
    subsystem: String,
    vars: UeventVars,
}

impl PreparedUevent {
    /// Creates a prepared event without validating it or allocating a sequence number.
    pub(crate) fn new(
        action: UeventAction,
        devpath: String,
        subsystem: String,
        vars: UeventVars,
    ) -> Self {
        Self {
            action,
            devpath,
            subsystem,
            vars,
        }
    }
}

/// A validated uevent with the sequence number assigned to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Uevent {
    action: UeventAction,
    devpath: String,
    subsystem: String,
    vars: UeventVars,
    seqnum: u64,
}

impl Uevent {
    /// Validates the prepared environment and allocates its sequence number.
    ///
    /// Every field before `SEQNUM` is counted before the counter is incremented,
    /// so only a failure to append `SEQNUM` itself leaves a gap in the sequence.
    ///
    /// Returns [`Error::InvalidValue`] if the path is relative, names `/sys`,
    /// or contains NUL, or if the subsystem is empty or contains NUL. Returns
    /// [`Error::NoMemory`] if the environment exceeds 64 entries or 2048 bytes.
    pub(crate) fn new(prepared: PreparedUevent) -> Result<Self> {
        let PreparedUevent {
            action,
            devpath,
            subsystem,
            vars,
        } = prepared;
        if !devpath.starts_with('/')
            || devpath == "/sys"
            || devpath.starts_with("/sys/")
            || devpath.contains('\0')
            || subsystem.is_empty()
            || subsystem.contains('\0')
        {
            return Err(Error::InvalidValue);
        }
        if vars.len() > MAX_UEVENT_VARS - 4 {
            return Err(Error::NoMemory);
        }
        let base_bytes = vars
            .total_bytes()
            .checked_add("ACTION".len() + action.as_str().len() + 2)
            .and_then(|bytes| bytes.checked_add("DEVPATH".len() + 2))
            .and_then(|bytes| bytes.checked_add(devpath.len()))
            .and_then(|bytes| bytes.checked_add("SUBSYSTEM".len() + 2))
            .and_then(|bytes| bytes.checked_add(subsystem.len()))
            .ok_or(Error::NoMemory)?;
        if base_bytes > MAX_UEVENT_BYTES {
            return Err(Error::NoMemory);
        }
        // Match Linux: allocate only after the fields that precede `SEQNUM` fit.
        let seqnum = CURRENT_SEQNUM
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let seq_bytes = "SEQNUM".len() + decimal_digits(seqnum) + 2;
        if base_bytes > MAX_UEVENT_BYTES - seq_bytes {
            // `SEQNUM` itself no longer fits; the number is already consumed.
            return Err(Error::NoMemory);
        }
        Ok(Self {
            action,
            devpath,
            subsystem,
            vars,
            seqnum,
        })
    }

    /// Returns the action.
    pub fn action(&self) -> UeventAction {
        self.action
    }

    /// Returns the sysfs device path encoded in `DEVPATH`, without `/sys`.
    pub fn devpath(&self) -> &str {
        &self.devpath
    }

    /// Returns the bus or class name encoded in `SUBSYSTEM`.
    pub fn subsystem(&self) -> &str {
        &self.subsystem
    }

    /// Returns the additional environment entries.
    pub fn vars(&self) -> &UeventVars {
        &self.vars
    }

    /// Returns the sequence number encoded in `SEQNUM`.
    pub fn seqnum(&self) -> u64 {
        self.seqnum
    }
}

fn decimal_digits(mut value: u64) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

/// Parses a request written to a device's sysfs `uevent` attribute.
///
/// The accepted syntax is `<action> [<UUID> [<KEY=VALUE> ...]]`. Exactly one
/// ASCII space separates items; leading, trailing, or repeated spaces are
/// rejected. One final `\n` or NUL is allowed. The optional UUID has the
/// 36-character `8-4-4-4-12` form and is preserved verbatim as
/// `SYNTH_UUID`; when it is absent, `SYNTH_UUID=0` is used. Each argument
/// must be an ASCII alphanumeric `KEY=VALUE` pair, and becomes
/// `SYNTH_ARG_KEY=VALUE` in input order. Duplicate keys are kept.
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
    let (action, tail) = request
        .split_once(' ')
        .map_or((request, None), |(action, tail)| (action, Some(tail)));
    let action = UeventAction::parse(action).ok_or(Error::InvalidValue)?;
    let mut vars = UeventVars::new();
    let Some(tail) = tail else {
        vars.add("SYNTH_UUID", "0")?;
        return Ok((action, vars));
    };
    let (uuid, args) = tail
        .split_once(' ')
        .map_or((tail, None), |(uuid, args)| (uuid, Some(args)));
    if !is_valid_uuid(uuid) {
        return Err(Error::InvalidValue);
    }
    vars.add("SYNTH_UUID", uuid)
        .map_err(|_| Error::InvalidValue)?;
    if let Some(args) = args {
        for token in args.split(' ') {
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

#[cfg(ktest)]
mod test {
    use alloc::{string::ToString, vec, vec::Vec};

    use ostd::prelude::ktest;

    use super::{PreparedUevent, Uevent, UeventAction, UeventVars};
    use crate::{common::Error, uevent};

    #[ktest]
    fn synthetic_environment_preserves_uuid_and_duplicate_argument_order() {
        let (action, vars) = uevent::parse_synthetic(
            "change 12345678-ABCD-1234-abcd-123456789abc FOO=1 BAR=2 FOO=3\0",
        )
        .unwrap();
        assert_eq!(action, UeventAction::Change);
        assert_eq!(
            vars.iter().collect::<Vec<_>>(),
            vec![
                ("SYNTH_UUID", "12345678-ABCD-1234-abcd-123456789abc"),
                ("SYNTH_ARG_FOO", "1"),
                ("SYNTH_ARG_BAR", "2"),
                ("SYNTH_ARG_FOO", "3"),
            ]
        );
        let (action, vars) = uevent::parse_synthetic("add\n").unwrap();
        assert_eq!(action, UeventAction::Add);
        assert_eq!(vars.iter().collect::<Vec<_>>(), vec![("SYNTH_UUID", "0")]);
    }

    #[ktest]
    fn variable_validation_does_not_modify_an_existing_environment() {
        let mut vars = UeventVars::new();
        vars.add("KEEP", "value").unwrap();
        let expected = vars.clone();
        for (key, value) in [("", "x"), ("K=V", "x"), ("K\0", "x"), ("K", "x\0y")] {
            assert!(matches!(vars.add(key, value), Err(Error::InvalidValue)));
            assert_eq!(vars, expected);
        }
        assert!(matches!(
            vars.add("BIG", "v".repeat(2048)),
            Err(Error::NoMemory)
        ));
        assert_eq!(vars, expected);
    }
    #[ktest]
    fn complete_events_count_base_fields_and_preserve_failed_append_state() {
        let mut vars = UeventVars::new();
        vars.add("K", "v".repeat(2045)).unwrap();
        assert_eq!(vars.total_bytes(), 2048);
        let snapshot = vars.clone();
        assert!(matches!(vars.add("NEXT", "v"), Err(Error::NoMemory)));
        assert_eq!(vars, snapshot);
        let before = uevent::current_seqnum();
        assert!(matches!(
            Uevent::new(PreparedUevent::new(
                UeventAction::Change,
                "/devices/budget".into(),
                "test".into(),
                vars
            )),
            Err(Error::NoMemory)
        ));
        assert_eq!(uevent::current_seqnum(), before);

        let mut vars = UeventVars::new();
        for _ in 0..60 {
            vars.add("K", "v").unwrap();
        }
        Uevent::new(PreparedUevent::new(
            UeventAction::Change,
            "/devices/budget".into(),
            "test".into(),
            vars.clone(),
        ))
        .unwrap();
        vars.add("K", "v").unwrap();
        let before = uevent::current_seqnum();
        assert!(matches!(
            Uevent::new(PreparedUevent::new(
                UeventAction::Change,
                "/devices/budget".into(),
                "test".into(),
                vars
            )),
            Err(Error::NoMemory)
        ));
        assert_eq!(uevent::current_seqnum(), before);

        let seq_digits = (before + 1).to_string().len();
        let base_bytes = "ACTION=change\0DEVPATH=/devices/budget\0SUBSYSTEM=test\0".len();
        let value_len = 2048 - base_bytes - "SEQNUM=\0".len() - seq_digits - "K=\0".len();
        let mut vars = UeventVars::new();
        vars.add("K", "v".repeat(value_len)).unwrap();
        let event = Uevent::new(PreparedUevent::new(
            UeventAction::Change,
            "/devices/budget".into(),
            "test".into(),
            vars,
        ))
        .unwrap();
        assert_eq!(event.seqnum(), before + 1);
        assert_eq!(
            base_bytes
                + "SEQNUM=\0".len()
                + event.seqnum().to_string().len()
                + event.vars().total_bytes(),
            2048
        );
        let mut oversized = UeventVars::new();
        oversized.add("K", "v".repeat(value_len + 1)).unwrap();
        let before = uevent::current_seqnum();
        assert!(matches!(
            Uevent::new(PreparedUevent::new(
                UeventAction::Change,
                "/devices/budget".into(),
                "test".into(),
                oversized
            )),
            Err(Error::NoMemory)
        ));
        assert_eq!(
            uevent::current_seqnum(),
            before + 1,
            "the final SEQNUM append consumes a sequence number and leaves a gap"
        );
    }
}
