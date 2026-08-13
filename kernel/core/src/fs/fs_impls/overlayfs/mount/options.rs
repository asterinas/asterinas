// SPDX-License-Identifier: MPL-2.0

//! Mount option parsing for overlayfs.
//!
//! This module validates the mount option string into an immutable
//! [`OverlayMountOptions`] construction input. The recognized key set is
//! closed ([`MountOptionKey`]); unknown keys fail with `EINVAL` before any
//! layer state is created.

use crate::{fs::vfs::file_system::FsFlags, prelude::*};

/// A recognized overlayfs mount option key.
///
/// The set is closed: any other key is rejected with `EINVAL`. Future option
/// keys must extend this enum instead of parsing ad-hoc strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MountOptionKey {
    /// The `lowerdir` option, a colon-separated list of lower layer paths.
    LowerDir,
    /// The `upperdir` option; absent on read-only overlays.
    UpperDir,
    /// The `workdir` option; required iff `upperdir` is present.
    WorkDir,
    /// The `uuid` option with values `off|null|on|auto`.
    Uuid,
    /// The `default_permissions` boolean option.
    DefaultPermissions,
    /// The `xino` option with values `off|auto|on`.
    Xino,
}

/// The UUID/`fsid` policy of an overlay mount.
///
/// The closed value set is `off|null|on|auto`; the default is
/// [`UuidMode::Auto`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UuidMode {
    /// The overlay UUID is null and the fsid comes from the topmost underlying fs.
    Off,
    /// Same as [`UuidMode::Off`], plus underlying-layer UUIDs are ignored.
    Null,
    /// The overlay UUID is generated and persisted as `trusted.overlay.uuid`.
    On,
    /// Reuse an existing persisted UUID or upgrade to `On`; degrade to `Null`.
    Auto,
}

/// The `xino=` mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) enum XinoMode {
    /// xino encoding disabled; non-directories report the underlying dev/ino.
    Off,
    /// xino enabled when feasible (the default).
    Auto,
    /// xino encoding always enabled.
    On,
}

/// Validated construction input for an overlay mount.
///
/// Fields are immutable after parsing. The struct is constructed once by
/// [`OverlayMountOptions::parse`], consumed once by `OverlayFs::new`, and
/// needs no lock because parsing happens in the single-threaded mount phase.
/// Invariants: `lower_dirs` is non-empty and
/// `upper_dir.is_some() == work_dir.is_some()`. The sibling `build.rs` reads
/// the fields directly as immutable construction inputs.
#[derive(Debug)]
pub(super) struct OverlayMountOptions {
    /// Lower layer paths in option order; the first option is the topmost
    /// lower layer (Linux `lowerdir=/l1:/l2:/l3` stacks `l1` topmost).
    pub(super) lower_dirs: Vec<String>,
    /// Upper layer path; `None` means a read-only overlay.
    pub(super) upper_dir: Option<String>,
    /// Work directory path; `Some` iff `upper_dir` is `Some`.
    pub(super) work_dir: Option<String>,
    /// Whether the mount was requested with `FsFlags::RDONLY`.
    pub(super) is_forced_read_only: bool,
    /// Whether the `default_permissions` option was set.
    pub(super) is_default_permissions: bool,
    /// The UUID persistence mode; defaults to [`UuidMode::Auto`].
    pub(super) uuid_mode: UuidMode,
    /// The `xino=` mode; defaults to [`XinoMode::Auto`].
    pub(super) xino_mode: XinoMode,
}

impl OverlayMountOptions {
    /// Parses the mount option string and mirror flags into validated options.
    ///
    /// Recognized keys are `lowerdir`, `upperdir`, `workdir`, `uuid`, `xino`,
    /// and `default_permissions`. Unknown keys, malformed `key=value` tokens,
    /// duplicate keys, missing `lowerdir`, empty values, and invalid `uuid`/
    /// `xino` values all fail with `EINVAL` before any layer state is created.
    pub(super) fn parse(args: Option<&str>, fs_flags: FsFlags) -> Result<Self> {
        let mut lower_dirs = Vec::new();
        let mut upper_dir = None;
        let mut work_dir = None;
        let mut is_default_permissions = false;
        let mut uuid_mode = UuidMode::Auto;
        // Each key may appear at most once; the tracked fields double as
        // seen-markers because they are assigned at most once. `uuid` needs a
        // dedicated marker because `UuidMode::Auto` is also the default value.
        let mut saw_uuid = false;
        // `xino` needs a dedicated marker because `XinoMode::Auto` is also
        // the default value (same discipline as `uuid`).
        let mut xino_mode = XinoMode::Auto;
        let mut saw_xino = false;

        let Some(args) = args else {
            return_errno_with_message!(
                Errno::EINVAL,
                "the `lowerdir` mount option must be specified"
            );
        };
        for entry in args.split(',') {
            if entry.is_empty() {
                continue;
            }
            let (key_name, value) = match entry.split_once('=') {
                Some((key_name, value)) => (key_name, Some(value)),
                None => (entry, None),
            };
            let key = match key_name {
                "lowerdir" => MountOptionKey::LowerDir,
                "upperdir" => MountOptionKey::UpperDir,
                "workdir" => MountOptionKey::WorkDir,
                "uuid" => MountOptionKey::Uuid,
                "xino" => MountOptionKey::Xino,
                "default_permissions" => MountOptionKey::DefaultPermissions,
                _ => {
                    return_errno_with_message!(Errno::EINVAL, "unknown overlay mount option");
                }
            };
            match key {
                MountOptionKey::LowerDir => {
                    if !lower_dirs.is_empty() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `lowerdir`"
                        );
                    }
                    let Some(value) = value else {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `lowerdir` mount option requires a value"
                        );
                    };
                    if value.is_empty() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `lowerdir` mount option requires a non-empty value"
                        );
                    }
                    // A single colon-joined `lowerdir` value is stacked
                    // left-to-right: the first path is the topmost layer
                    // (Linux multi-layer semantics).
                    lower_dirs = value.split(':').map(str::to_string).collect();
                    if lower_dirs.iter().any(|lower_dir| lower_dir.is_empty()) {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `lowerdir` value contains an empty layer path"
                        );
                    }
                }
                MountOptionKey::UpperDir => {
                    if upper_dir.is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `upperdir`"
                        );
                    }
                    let Some(value) = value else {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `upperdir` mount option requires a value"
                        );
                    };
                    if value.is_empty() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `upperdir` mount option requires a non-empty value"
                        );
                    }
                    upper_dir = Some(value.to_string());
                }
                MountOptionKey::WorkDir => {
                    if work_dir.is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `workdir`"
                        );
                    }
                    let Some(value) = value else {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `workdir` mount option requires a value"
                        );
                    };
                    if value.is_empty() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `workdir` mount option requires a non-empty value"
                        );
                    }
                    work_dir = Some(value.to_string());
                }
                MountOptionKey::Uuid => {
                    if saw_uuid {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `uuid`"
                        );
                    }
                    saw_uuid = true;
                    let Some(value) = value else {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `uuid` mount option requires a value"
                        );
                    };
                    uuid_mode = match value {
                        "off" => UuidMode::Off,
                        "null" => UuidMode::Null,
                        "on" => UuidMode::On,
                        "auto" => UuidMode::Auto,
                        _ => {
                            return_errno_with_message!(
                                Errno::EINVAL,
                                "invalid `uuid` mount option value"
                            );
                        }
                    };
                }
                MountOptionKey::Xino => {
                    if saw_xino {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `xino`"
                        );
                    }
                    saw_xino = true;
                    let Some(value) = value else {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `xino` mount option requires a value"
                        );
                    };
                    xino_mode = match value {
                        "off" => XinoMode::Off,
                        "auto" => XinoMode::Auto,
                        "on" => XinoMode::On,
                        _ => {
                            return_errno_with_message!(
                                Errno::EINVAL,
                                "invalid `xino` mount option value"
                            );
                        }
                    };
                }
                MountOptionKey::DefaultPermissions => {
                    if is_default_permissions {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `default_permissions`"
                        );
                    }
                    if value.is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `default_permissions` mount option does not take a value"
                        );
                    }
                    is_default_permissions = true;
                }
            }
        }

        if lower_dirs.is_empty() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the `lowerdir` mount option must be specified"
            );
        }
        if upper_dir.is_some() != work_dir.is_some() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the `workdir` mount option is required iff `upperdir` is specified"
            );
        }

        Ok(Self {
            lower_dirs,
            upper_dir,
            work_dir,
            is_forced_read_only: fs_flags.contains(FsFlags::RDONLY),
            is_default_permissions,
            uuid_mode,
            xino_mode,
        })
    }
}
