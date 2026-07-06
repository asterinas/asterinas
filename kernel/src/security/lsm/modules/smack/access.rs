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

    /// Returns the canonical rule text for this access set.
    pub fn as_rule_text(self) -> String {
        let mut text = String::with_capacity(6);
        for (access, name) in [
            (Self::READ, 'r'),
            (Self::WRITE, 'w'),
            (Self::EXECUTE, 'x'),
            (Self::APPEND, 'a'),
            (Self::TRANSMUTE, 't'),
            (Self::BRINGUP, 'b'),
        ] {
            if self.contains(access) {
                text.push(name);
            }
        }

        if text.is_empty() {
            text.push('-');
        }

        text
    }
}

/// Adds or replaces a Smack access rule.
pub fn set_rule(subject: SmackLabel, object: SmackLabel, access: SmackAccess) {
    access_rules()
        .write()
        .insert(RuleKey { subject, object }, access);
}

/// Loads newline-delimited Smack access rules.
pub fn load_rules(policy: &str) -> Result<usize> {
    let mut parsed_rules = Vec::new();
    for line in policy.lines() {
        let Some(rule) = parse_rule_line(line)? else {
            continue;
        };
        parsed_rules.push(rule);
    }

    let parsed_rule_count = parsed_rules.len();
    for rule in parsed_rules {
        set_rule(rule.subject, rule.object, rule.access);
    }

    Ok(parsed_rule_count)
}

/// Returns all loaded Smack access rules in deterministic order.
pub fn rules_as_text() -> String {
    let mut text = String::new();
    let rules = access_rules().read();
    for (key, access) in rules.iter() {
        text.push_str(key.subject.as_str());
        text.push(' ');
        text.push_str(key.object.as_str());
        text.push(' ');
        text.push_str(&access.as_rule_text());
        text.push('\n');
    }

    text
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

struct ParsedRule {
    subject: SmackLabel,
    object: SmackLabel,
    access: SmackAccess,
}

fn parse_rule_line(line: &str) -> Result<Option<ParsedRule>> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return Ok(None);
    }

    let mut fields = line.split_whitespace();
    let Some(subject) = fields.next() else {
        return Ok(None);
    };
    let Some(object) = fields.next() else {
        return_errno_with_message!(Errno::EINVAL, "the Smack rule object label is missing");
    };
    let Some(access) = fields.next() else {
        return_errno_with_message!(Errno::EINVAL, "the Smack rule access mode is missing");
    };
    if fields.next().is_some() {
        return_errno_with_message!(Errno::EINVAL, "the Smack rule has too many fields");
    }

    Ok(Some(ParsedRule {
        subject: SmackLabel::parse(subject)?,
        object: SmackLabel::parse(object)?,
        access: SmackAccess::parse(access)?,
    }))
}

fn access_rules() -> &'static RwMutex<BTreeMap<RuleKey, SmackAccess>> {
    ACCESS_RULES.call_once(|| RwMutex::new(BTreeMap::new()))
}
