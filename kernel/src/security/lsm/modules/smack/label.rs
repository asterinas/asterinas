// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// The maximum size of a Smack label.
pub const MAX_LABEL_LEN: usize = 255;

/// A validated Smack label.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SmackLabel(String);

impl SmackLabel {
    /// Creates a Smack label from validated text.
    pub fn parse(label: &str) -> Result<Self> {
        validate_label(label)?;
        Ok(Self(label.to_string()))
    }

    /// Creates a Smack label from an xattr value.
    pub fn parse_xattr_value(value: &[u8]) -> Result<Self> {
        let label = core::str::from_utf8(value)
            .map_err(|_| Error::with_message(Errno::EINVAL, "the Smack label is not UTF-8"))?;
        Self::parse(label)
    }

    /// Creates the Smack floor label.
    pub fn floor() -> Self {
        Self("_".to_string())
    }

    /// Returns the label text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether this is the floor label.
    pub fn is_floor(&self) -> bool {
        self.as_str() == "_"
    }

    /// Returns whether this is the hat label.
    pub fn is_hat(&self) -> bool {
        self.as_str() == "^"
    }

    /// Returns whether this is the star label.
    pub fn is_star(&self) -> bool {
        self.as_str() == "*"
    }
}

impl Default for SmackLabel {
    fn default() -> Self {
        Self::floor()
    }
}

fn validate_label(label: &str) -> Result<()> {
    if label.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the Smack label is empty");
    }
    if label.len() > MAX_LABEL_LEN {
        return_errno_with_message!(Errno::EINVAL, "the Smack label is too long");
    }
    if label.starts_with('-') {
        return_errno_with_message!(
            Errno::EINVAL,
            "the Smack label starts with a reserved prefix"
        );
    }

    for byte in label.bytes() {
        if !byte.is_ascii_graphic() || matches!(byte, b'/' | b'\\' | b'\'' | b'"') {
            return_errno_with_message!(Errno::EINVAL, "the Smack label contains invalid bytes");
        }
    }

    Ok(())
}
