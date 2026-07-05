// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;
use spin::Once;

use super::label::SmackLabel;
use crate::prelude::*;

bitflags! {
    /// Smack access modes.
    pub struct SmackAccess: u8 {
        /// Read access.
        const READ = 1 << 0;
        /// Write access.
        const WRITE = 1 << 1;
        /// Execute access.
        const EXECUTE = 1 << 2;
        /// Append access.
        const APPEND = 1 << 3;
        /// Transmute access.
        const TRANSMUTE = 1 << 4;
        /// Bring-up reporting.
        const BRINGUP = 1 << 5;
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RuleKey {
    subject: SmackLabel,
    object: SmackLabel,
}

static ACCESS_RULES: Once<RwMutex<BTreeMap<RuleKey, SmackAccess>>> = Once::new();

impl SmackAccess {
    /// Parses a Smack access string.
    #[expect(dead_code, reason = "Smack rule loading will use this parser.")]
    pub fn parse(access: &str) -> Result<Self> {
        let mut parsed = Self::empty();
        for byte in access.bytes() {
            match byte.to_ascii_lowercase() {
                b'r' => parsed |= Self::READ,
                b'w' => parsed |= Self::WRITE,
                b'x' => parsed |= Self::EXECUTE,
                b'a' => parsed |= Self::APPEND,
                b't' => parsed |= Self::TRANSMUTE,
                b'b' => parsed |= Self::BRINGUP,
                b'-' => {}
                _ => return_errno_with_message!(Errno::EINVAL, "the Smack access mode is invalid"),
            }
        }

        Ok(parsed)
    }
}

/// Adds or replaces a Smack access rule.
#[expect(
    dead_code,
    reason = "Smack rule loading will install policy with this helper."
)]
pub fn set_rule(subject: SmackLabel, object: SmackLabel, access: SmackAccess) {
    access_rules()
        .write()
        .insert(RuleKey { subject, object }, access);
}

/// Checks whether a Smack subject can access an object.
pub fn check(subject: &SmackLabel, object: &SmackLabel, requested: SmackAccess) -> Result<()> {
    if is_allowed(subject, object, requested) {
        return Ok(());
    }

    return_errno_with_message!(
        Errno::EACCES,
        "Smack access rules deny the requested access"
    );
}

fn is_allowed(subject: &SmackLabel, object: &SmackLabel, requested: SmackAccess) -> bool {
    if requested.is_empty() {
        return true;
    }
    if subject.is_star() {
        return false;
    }
    let read_execute = SmackAccess::READ | SmackAccess::EXECUTE;
    if subject.is_hat() && read_execute.contains(requested) {
        return true;
    }
    if object.is_floor() && read_execute.contains(requested) {
        return true;
    }
    if object.is_star() {
        return true;
    }
    if subject == object {
        return true;
    }

    let rules = access_rules().read();
    let key = RuleKey {
        subject: subject.clone(),
        object: object.clone(),
    };
    rules
        .get(&key)
        .is_some_and(|allowed| allowed.contains(requested))
}

fn access_rules() -> &'static RwMutex<BTreeMap<RuleKey, SmackAccess>> {
    ACCESS_RULES.call_once(|| RwMutex::new(BTreeMap::new()))
}
