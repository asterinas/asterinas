// SPDX-License-Identifier: MPL-2.0

use alloc::format;

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
static ACCESS_QUERY_RESULT: Once<RwMutex<Option<bool>>> = Once::new();
static AMBIENT_LABEL: Once<RwMutex<SmackLabel>> = Once::new();
static ONLYCAP_LABELS: Once<RwMutex<BTreeSet<SmackLabel>>> = Once::new();
static LOGGING_MODE: Once<RwMutex<SmackLoggingMode>> = Once::new();

/// Smack access logging mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmackLoggingMode {
    /// Logs no access decisions.
    None,
    /// Logs denied access decisions.
    Denied,
    /// Logs accepted access decisions.
    Accepted,
    /// Logs denied and accepted access decisions.
    Both,
}

impl SmackLoggingMode {
    /// Parses a Smack logging mode.
    pub fn parse(mode: &str) -> Result<Self> {
        match mode.trim() {
            "0" => Ok(Self::None),
            "1" => Ok(Self::Denied),
            "2" => Ok(Self::Accepted),
            "3" => Ok(Self::Both),
            _ => return_errno_with_message!(Errno::EINVAL, "the Smack logging mode is invalid"),
        }
    }

    /// Returns the Linux-compatible numeric text for this logging mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "0",
            Self::Denied => "1",
            Self::Accepted => "2",
            Self::Both => "3",
        }
    }

    fn logs_denied(self) -> bool {
        matches!(self, Self::Denied | Self::Both)
    }

    fn logs_accepted(self) -> bool {
        matches!(self, Self::Accepted | Self::Both)
    }
}

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

/// Enables and disables access bits in one Smack access rule from text.
pub fn change_rule(rule: &str) -> Result<()> {
    let Some(rule) = parse_rule_change_line(rule)? else {
        return_errno_with_message!(Errno::EINVAL, "the Smack rule is empty");
    };

    let mut rules = access_rules().write();
    let access = rules.entry(rule.key).or_insert(SmackAccess::empty());
    access.insert(rule.enabled);
    access.remove(rule.disabled);
    Ok(())
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

/// Removes all access rules whose subject is `subject`.
pub fn revoke_subject(subject: &str) -> Result<usize> {
    let subject = SmackLabel::parse(subject.trim())?;
    let mut rules = access_rules().write();
    let previous_rule_count = rules.len();
    rules.retain(|key, _| key.subject != subject);
    Ok(previous_rule_count - rules.len())
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

/// Checks an access query and stores the result for `/proc/smack/access`.
pub fn query_access(query: &str) -> Result<bool> {
    let Some(rule) = parse_rule_line(query)? else {
        return_errno_with_message!(Errno::EINVAL, "the Smack access query is empty");
    };

    let is_allowed = is_allowed(&rule.subject, &rule.object, rule.access);
    *access_query_result().write() = Some(is_allowed);
    Ok(is_allowed)
}

/// Returns the last access query result as Linux-compatible text.
pub fn access_query_result_as_text() -> String {
    match *access_query_result().read() {
        Some(true) => "1\n".to_string(),
        Some(false) => "0\n".to_string(),
        None => String::new(),
    }
}

/// Returns the current ambient label.
pub fn ambient_label() -> SmackLabel {
    ambient_label_lock().read().clone()
}

/// Returns the current ambient label as text.
pub fn ambient_label_as_text() -> String {
    let label = ambient_label_lock().read();
    format!("{}\n", label.as_str())
}

/// Sets the current ambient label.
pub fn set_ambient_label(label: &str) -> Result<()> {
    *ambient_label_lock().write() = SmackLabel::parse(label.trim())?;
    Ok(())
}

/// Returns the current `onlycap` labels as text.
pub fn onlycap_labels_as_text() -> String {
    let labels = onlycap_labels().read();
    labels
        .iter()
        .map(|label| label.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        + if labels.is_empty() { "" } else { "\n" }
}

/// Replaces the current `onlycap` label set.
pub fn set_onlycap_labels(labels: &str) -> Result<()> {
    *onlycap_labels().write() = parse_label_set(labels)?;
    Ok(())
}

/// Checks whether `subject` is allowed by the current `onlycap` policy.
pub fn subject_has_onlycap(subject: &SmackLabel) -> bool {
    let labels = onlycap_labels().read();
    labels.is_empty() || labels.contains(subject)
}

/// Returns the current logging mode as text.
pub fn logging_mode_as_text() -> String {
    let logging_mode = *logging_mode().read();
    format!("{}\n", logging_mode.as_str())
}

/// Sets the current logging mode.
pub fn set_logging_mode(mode: &str) -> Result<()> {
    *logging_mode().write() = SmackLoggingMode::parse(mode)?;
    Ok(())
}

/// Checks whether a Smack subject can access an object.
pub fn check(subject: &SmackLabel, object: &SmackLabel, requested: SmackAccess) -> Result<()> {
    if is_allowed(subject, object, requested) {
        log_access_decision(subject, object, requested, true);
        return Ok(());
    }

    log_access_decision(subject, object, requested, false);
    return_errno_with_message!(
        Errno::EACCES,
        "Smack access rules deny the requested access"
    );
}

/// Returns whether a Smack access request is allowed.
pub fn is_allowed(subject: &SmackLabel, object: &SmackLabel, requested: SmackAccess) -> bool {
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

fn log_access_decision(
    subject: &SmackLabel,
    object: &SmackLabel,
    requested: SmackAccess,
    allowed: bool,
) {
    let logging_mode = *logging_mode().read();
    if allowed && !logging_mode.logs_accepted() {
        return;
    }
    if !allowed && !logging_mode.logs_denied() {
        return;
    }

    info!(
        "smack access subject={} object={} request={} result={}",
        subject.as_str(),
        object.as_str(),
        requested.as_rule_text(),
        if allowed { "allow" } else { "deny" },
    );
}

struct ParsedRule {
    subject: SmackLabel,
    object: SmackLabel,
    access: SmackAccess,
}

struct ParsedRuleChange {
    key: RuleKey,
    enabled: SmackAccess,
    disabled: SmackAccess,
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

fn parse_rule_change_line(line: &str) -> Result<Option<ParsedRuleChange>> {
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
    let Some(enabled) = fields.next() else {
        return_errno_with_message!(Errno::EINVAL, "the Smack rule enabled access is missing");
    };
    let Some(disabled) = fields.next() else {
        return_errno_with_message!(Errno::EINVAL, "the Smack rule disabled access is missing");
    };
    if fields.next().is_some() {
        return_errno_with_message!(Errno::EINVAL, "the Smack rule has too many fields");
    }

    Ok(Some(ParsedRuleChange {
        key: RuleKey {
            subject: SmackLabel::parse(subject)?,
            object: SmackLabel::parse(object)?,
        },
        enabled: SmackAccess::parse(enabled)?,
        disabled: SmackAccess::parse(disabled)?,
    }))
}

fn parse_label_set(labels: &str) -> Result<BTreeSet<SmackLabel>> {
    let labels = labels.trim();
    if labels.is_empty() || labels == "-" {
        return Ok(BTreeSet::new());
    }

    labels
        .split_whitespace()
        .map(SmackLabel::parse)
        .collect::<Result<_>>()
}

fn access_rules() -> &'static RwMutex<BTreeMap<RuleKey, SmackAccess>> {
    ACCESS_RULES.call_once(|| RwMutex::new(BTreeMap::new()))
}

fn access_query_result() -> &'static RwMutex<Option<bool>> {
    ACCESS_QUERY_RESULT.call_once(|| RwMutex::new(None))
}

fn ambient_label_lock() -> &'static RwMutex<SmackLabel> {
    AMBIENT_LABEL.call_once(|| RwMutex::new(SmackLabel::floor()))
}

fn onlycap_labels() -> &'static RwMutex<BTreeSet<SmackLabel>> {
    ONLYCAP_LABELS.call_once(|| RwMutex::new(BTreeSet::new()))
}

fn logging_mode() -> &'static RwMutex<SmackLoggingMode> {
    LOGGING_MODE.call_once(|| RwMutex::new(SmackLoggingMode::Denied))
}
