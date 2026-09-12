// SPDX-License-Identifier: MPL-2.0

//! Device numbers and `/dev` node requests.

use core::fmt;

use device_id::DeviceId;

use crate::SysStr;

/// Whether a device number names a character or a block device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevKind {
    Char,
    Block,
}

/// A device number together with its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevNum {
    kind: DevKind,
    id: DeviceId,
}

impl DevNum {
    /// Creates a character device number.
    pub fn char(id: DeviceId) -> Self {
        Self {
            kind: DevKind::Char,
            id,
        }
    }

    /// Creates a block device number.
    pub fn block(id: DeviceId) -> Self {
        Self {
            kind: DevKind::Block,
            id,
        }
    }

    /// Returns the kind.
    pub fn kind(&self) -> DevKind {
        self.kind
    }

    /// Returns the major and minor number.
    pub fn id(&self) -> DeviceId {
        self.id
    }
}

impl fmt::Display for DevNum {
    /// Formats as `major:minor`, the form used by `/sys/dev` and the `dev`
    /// attribute.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.id.major().get(), self.id.minor().get())
    }
}

/// A request to create or delete a `/dev` node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevNodeRequest {
    /// The device number the node refers to.
    pub devnum: DevNum,
    /// The path of the node relative to `/dev`, e.g. `null` or `input/event0`.
    pub path: SysStr,
    /// The permission bits of the node.
    pub mode: u16,
}

/// The default mode of a device node when neither the device type nor the
/// class overrides it (Linux devtmpfs uses `0600` as well).
pub const DEFAULT_DEVNODE_MODE: u16 = 0o600;
