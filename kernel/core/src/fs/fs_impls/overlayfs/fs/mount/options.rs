// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Mount option parsing for overlayfs.
//!
//! This module validates the mount option string into a
//! [`MountOptions`] construction input. The recognized keys are the path
//! keys `lowerdir`, `lowerdir+`, `upperdir`, and `workdir`, the mode keys
//! `uuid` and `xino`, the raw-intent keys `redirect_dir`, `index`,
//! `nfs_export`, `metacopy`, `verity`, and `fsync`, and the valueless keys
//! `default_permissions`, `userxattr`, `volatile`, and `nooverride_creds`.
//!
//! Unknown keys fail with `EINVAL` before any layer state is created, as
//! do `datadir+` and `override_creds`. A raw-intent key records an
//! explicitly requested feature that is not implemented: the request is
//! accepted and disclosed as a one-shot mount-time degrade by the verify
//! phase.
//!
//! `override_creds` fails fast instead of silently inverting credential
//! semantics: the VFS exposes no credentials-stash/override surface. Accept
//! the option once the VFS provides that surface.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v7.0/source/Documentation/filesystems/overlayfs.rst#L350-L364>
//!   (Linux stacks colon-separated lowerdirs with the first entry topmost)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/params.c>
//!   (upstream option parse and `ovl_fs_params_verify`: key tables, value
//!   domains, the `volatile` alias for `fsync=volatile`, and the cross-key
//!   conflict rules)

use super::super::policy::{UuidMode, XinoMode};
use crate::{fs::vfs::file_system::FsFlags, prelude::*};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RedirectDirMode {
    Off,
    Follow,
    NoFollow,
    On,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum VerityMode {
    Off,
    On,
    Require,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FsyncMode {
    Volatile,
    Auto,
    Strict,
}

#[derive(Debug)]
pub(super) struct MountOptions {
    pub(super) lower_dirs: Vec<String>,
    pub(super) upper_dir: Option<String>,
    pub(super) work_dir: Option<String>,
    pub(super) is_forced_read_only: bool,
    pub(super) is_default_permissions: bool,
    pub(super) is_userxattr: bool,
    pub(super) uuid_mode: Option<UuidMode>,
    pub(super) xino_mode: Option<XinoMode>,
    redirect_dir: Option<RedirectDirMode>,
    index: Option<bool>,
    nfs_export: Option<bool>,
    metacopy: Option<bool>,
    verity: Option<VerityMode>,
    fsync_mode: Option<FsyncMode>,
}

impl MountOptions {
    pub(super) fn parse(args: Option<&str>, fs_flags: FsFlags) -> Result<Self> {
        let mut lower_dirs = Vec::new();
        let mut upper_dir = None;
        let mut work_dir = None;
        let mut is_default_permissions = false;
        let mut is_userxattr = false;
        let mut uuid_mode = None;
        let mut xino_mode = None;
        let mut redirect_dir = None;
        let mut index = None;
        let mut nfs_export = None;
        let mut metacopy = None;
        let mut verity = None;
        let mut fsync_mode = None;
        let mut is_lowerdir_seen = false;
        let mut is_lowerdir_plus_seen = false;
        let mut is_nooverride_creds_seen = false;

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
            match key_name {
                "lowerdir" | "lowerdir+" | "upperdir" | "workdir" | "uuid" | "xino"
                | "redirect_dir" | "index" | "nfs_export" | "metacopy" | "verity" | "fsync" => {
                    let Some(value) = value else {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `{key_name}` mount option requires a value"
                        );
                    };
                    match key_name {
                        "lowerdir" => {
                            if is_lowerdir_seen {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `lowerdir`"
                                );
                            }
                            if is_lowerdir_plus_seen {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "the `lowerdir+` mount option cannot be combined with `lowerdir`"
                                );
                            }
                            Self::require_non_empty_value(
                                value,
                                "the `lowerdir` mount option requires a non-empty value",
                            )?;
                            lower_dirs = value.split(':').map(str::to_string).collect();
                            if lower_dirs.iter().any(|lower_dir| lower_dir.is_empty()) {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "the `lowerdir` value contains an empty layer path"
                                );
                            }
                            is_lowerdir_seen = true;
                        }
                        "lowerdir+" => {
                            Self::require_non_empty_value(
                                value,
                                "the `lowerdir+` mount option requires a non-empty value",
                            )?;
                            if is_lowerdir_seen {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "the `lowerdir+` mount option cannot be combined with `lowerdir`"
                                );
                            }
                            lower_dirs.push(value.to_string());
                            is_lowerdir_plus_seen = true;
                        }
                        "upperdir" => {
                            if upper_dir.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `upperdir`"
                                );
                            }
                            Self::require_non_empty_value(
                                value,
                                "the `upperdir` mount option requires a non-empty value",
                            )?;
                            upper_dir = Some(value.to_string());
                        }
                        "workdir" => {
                            if work_dir.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `workdir`"
                                );
                            }
                            Self::require_non_empty_value(
                                value,
                                "the `workdir` mount option requires a non-empty value",
                            )?;
                            work_dir = Some(value.to_string());
                        }
                        "uuid" => {
                            if uuid_mode.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `uuid`"
                                );
                            }
                            uuid_mode = Some(match value {
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
                            });
                        }
                        "xino" => {
                            if xino_mode.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `xino`"
                                );
                            }
                            xino_mode = Some(match value {
                                "off" => XinoMode::Off,
                                "auto" => XinoMode::Auto,
                                "on" => XinoMode::On,
                                _ => {
                                    return_errno_with_message!(
                                        Errno::EINVAL,
                                        "invalid `xino` mount option value"
                                    );
                                }
                            });
                        }
                        "redirect_dir" => {
                            if redirect_dir.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `redirect_dir`"
                                );
                            }
                            redirect_dir = Some(match value {
                                "off" => RedirectDirMode::Off,
                                "follow" => RedirectDirMode::Follow,
                                "nofollow" => RedirectDirMode::NoFollow,
                                "on" => RedirectDirMode::On,
                                _ => {
                                    return_errno_with_message!(
                                        Errno::EINVAL,
                                        "invalid `redirect_dir` mount option value"
                                    );
                                }
                            });
                        }
                        "index" => {
                            if index.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `index`"
                                );
                            }
                            index = Some(match value {
                                "on" => true,
                                "off" => false,
                                _ => {
                                    return_errno_with_message!(
                                        Errno::EINVAL,
                                        "invalid `index` mount option value"
                                    );
                                }
                            });
                        }
                        "nfs_export" => {
                            if nfs_export.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `nfs_export`"
                                );
                            }
                            nfs_export = Some(match value {
                                "on" => true,
                                "off" => false,
                                _ => {
                                    return_errno_with_message!(
                                        Errno::EINVAL,
                                        "invalid `nfs_export` mount option value"
                                    );
                                }
                            });
                        }
                        "metacopy" => {
                            if metacopy.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `metacopy`"
                                );
                            }
                            metacopy = Some(match value {
                                "on" => true,
                                "off" => false,
                                _ => {
                                    return_errno_with_message!(
                                        Errno::EINVAL,
                                        "invalid `metacopy` mount option value"
                                    );
                                }
                            });
                        }
                        "verity" => {
                            if verity.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `verity`"
                                );
                            }
                            verity = Some(match value {
                                "off" => VerityMode::Off,
                                "on" => VerityMode::On,
                                "require" => VerityMode::Require,
                                _ => {
                                    return_errno_with_message!(
                                        Errno::EINVAL,
                                        "invalid `verity` mount option value"
                                    );
                                }
                            });
                        }
                        "fsync" => {
                            // Only `auto` is honored: data copy-up already syncs
                            // before publication.
                            if fsync_mode.is_some() {
                                return_errno_with_message!(
                                    Errno::EINVAL,
                                    "duplicate overlay mount option `fsync`"
                                );
                            }
                            fsync_mode = Some(match value {
                                "volatile" => FsyncMode::Volatile,
                                "auto" => FsyncMode::Auto,
                                "strict" => FsyncMode::Strict,
                                _ => {
                                    return_errno_with_message!(
                                        Errno::EINVAL,
                                        "invalid `fsync` mount option value"
                                    );
                                }
                            });
                        }
                        _ => unreachable!("key_name was filtered above"),
                    }
                }
                "default_permissions" => {
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
                "userxattr" => {
                    if is_userxattr {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `userxattr`"
                        );
                    }
                    if value.is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `userxattr` mount option does not take a value"
                        );
                    }
                    is_userxattr = true;
                }
                "volatile" => {
                    if fsync_mode.is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `volatile`"
                        );
                    }
                    if value.is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `volatile` mount option does not take a value"
                        );
                    }
                    fsync_mode = Some(FsyncMode::Volatile);
                }
                "nooverride_creds" => {
                    // Caller-credential checks are the only local permission model,
                    // so the negated form is accepted as a silent no-op.
                    if is_nooverride_creds_seen {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "duplicate overlay mount option `nooverride_creds`"
                        );
                    }
                    if value.is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the `nooverride_creds` mount option does not take a value"
                        );
                    }
                    is_nooverride_creds_seen = true;
                }
                "override_creds" => {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the `override_creds` mount option requires a VFS credentials API"
                    );
                }
                "datadir+" => {
                    // Data-only layers have no local concept; the only available
                    // degradation (a regular lower layer) would silently publish
                    // the data directory's entries in the merged view, so the
                    // option is rejected outright.
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the `datadir+` mount option is not supported (data-only layers are not implemented)"
                    );
                }
                _ => {
                    return_errno_with_message!(Errno::EINVAL, "unknown overlay mount option");
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
                "the `workdir` mount option is required if and only if `upperdir` is specified"
            );
        }

        let options = Self {
            lower_dirs,
            upper_dir,
            work_dir,
            is_forced_read_only: fs_flags.contains(FsFlags::RDONLY),
            is_default_permissions,
            is_userxattr,
            uuid_mode,
            xino_mode,
            redirect_dir,
            index,
            nfs_export,
            metacopy,
            verity,
            fsync_mode,
        };
        options.verify()?;
        Ok(options)
    }

    fn verify(&self) -> Result<()> {
        if self.is_userxattr {
            match self.redirect_dir {
                Some(RedirectDirMode::Off) => {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "conflicting overlay mount options: `userxattr` and `redirect_dir=off`"
                    );
                }
                Some(RedirectDirMode::Follow) => {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "conflicting overlay mount options: `userxattr` and `redirect_dir=follow`"
                    );
                }
                Some(RedirectDirMode::On) => {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "conflicting overlay mount options: `userxattr` and `redirect_dir=on`"
                    );
                }
                Some(RedirectDirMode::NoFollow) | None => {}
            }
            if self.metacopy == Some(true) {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "conflicting overlay mount options: `userxattr` and `metacopy=on`"
                );
            }
        }
        if self.metacopy == Some(true) {
            match self.redirect_dir {
                Some(RedirectDirMode::Off) => {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "conflicting overlay mount options: `metacopy=on` and `redirect_dir=off`"
                    );
                }
                Some(RedirectDirMode::Follow) => {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "conflicting overlay mount options: `metacopy=on` and `redirect_dir=follow`"
                    );
                }
                Some(RedirectDirMode::NoFollow) => {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "conflicting overlay mount options: `metacopy=on` and `redirect_dir=nofollow`"
                    );
                }
                Some(RedirectDirMode::On) | None => {}
            }
        }
        if self.nfs_export == Some(true) && self.index == Some(false) {
            return_errno_with_message!(
                Errno::EINVAL,
                "conflicting overlay mount options: `nfs_export=on` and `index=off`"
            );
        }
        if self.nfs_export == Some(true) && self.metacopy == Some(true) {
            return_errno_with_message!(
                Errno::EINVAL,
                "conflicting overlay mount options: `nfs_export=on` and `metacopy=on`"
            );
        }

        match self.redirect_dir {
            Some(RedirectDirMode::Off) => {
                warn!(
                    "`redirect_dir=off`: degrading to redirect_dir=nofollow; directory redirects are neither recorded nor followed"
                );
            }
            Some(RedirectDirMode::Follow) => {
                warn!(
                    "`redirect_dir=follow`: degrading to redirect_dir=nofollow; directory redirects are neither recorded nor followed"
                );
            }
            Some(RedirectDirMode::On) => {
                warn!(
                    "`redirect_dir=on`: degrading to redirect_dir=nofollow; directory redirects are neither recorded nor followed"
                );
            }
            Some(RedirectDirMode::NoFollow) | None => {}
        }
        if self.index == Some(true) {
            warn!("`index=on`: degrading to index=off; no inode index is maintained");
        }
        if self.nfs_export == Some(true) {
            warn!(
                "`nfs_export=on`: degrading to nfs_export=off; no export file handles are encoded"
            );
        }
        if self.metacopy == Some(true) {
            warn!("`metacopy=on`: degrading to metacopy=off; copy-up always copies data");
        }
        match self.verity {
            Some(VerityMode::On) => {
                warn!("`verity=on`: degrading to verity=off; fs-verity digests are not enforced");
            }
            Some(VerityMode::Require) => {
                warn!(
                    "`verity=require`: degrading to verity=off; fs-verity digests are not enforced"
                );
            }
            Some(VerityMode::Off) | None => {}
        }
        match self.fsync_mode {
            Some(FsyncMode::Strict) => {
                warn!("`fsync=strict`: metadata/directory copy-up is not explicitly synced");
            }
            Some(FsyncMode::Volatile) => {
                warn!(
                    "`fsync=volatile`: sync suppression, the volatile dirty marker, and sticky syncfs errors are not implemented; durability follows the underlying filesystem"
                );
            }
            Some(FsyncMode::Auto) | None => {}
        }
        Ok(())
    }

    fn require_non_empty_value(value: &str, message: &'static str) -> Result<()> {
        if value.is_empty() {
            return_errno_with_message!(Errno::EINVAL, message);
        }
        Ok(())
    }
}

#[cfg(ktest)]
mod test {
    // SPDX-License-Identifier: MPL-2.0

    //! Unit tests for the pure [`MountOptions::parse`] contract.
    //!
    //! The assertions form a fixed case table for the parse surface; the
    //! tests check parsing only: no filesystem, VFS, block, or I/O fixture
    //! is constructed.

    use ostd::prelude::ktest;

    use super::*;

    fn parse_expect_ok(args: &str, fs_flags: FsFlags) -> MountOptions {
        MountOptions::parse(Some(args), fs_flags).unwrap()
    }

    fn parse_expect_einval(args: Option<&str>, fs_flags: FsFlags) -> Error {
        let err = MountOptions::parse(args, fs_flags).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);
        err
    }

    #[ktest]
    fn parse_requires_lowerdir() {
        parse_expect_einval(None, FsFlags::empty());
        parse_expect_einval(Some(""), FsFlags::empty());
        parse_expect_einval(Some(",,"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir="), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=a::b"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=:"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=a:"), FsFlags::empty());
    }

    #[ktest]
    fn parse_lowerdir_layer_list() {
        let options = parse_expect_ok("lowerdir=a", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["a"]);
        let options = parse_expect_ok("lowerdir=a:b:c", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["a", "b", "c"]);
        let read_only = parse_expect_ok("lowerdir=l", FsFlags::RDONLY);
        assert!(read_only.is_forced_read_only);
        let writable = parse_expect_ok("lowerdir=l", FsFlags::empty());
        assert!(!writable.is_forced_read_only);
    }

    #[ktest]
    fn parse_upperdir_workdir_pairing() {
        parse_expect_einval(Some("lowerdir=l,upperdir=u"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,workdir=w"), FsFlags::empty());
        let options = parse_expect_ok("lowerdir=l,upperdir=u,workdir=w", FsFlags::empty());
        assert_eq!(options.upper_dir.as_deref(), Some("u"));
        assert_eq!(options.work_dir.as_deref(), Some("w"));
    }

    #[ktest]
    fn parse_rejects_duplicate_keys() {
        parse_expect_einval(Some("lowerdir=a,lowerdir=b"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,upperdir=u,upperdir=v"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,workdir=w,workdir=v"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,uuid=on,uuid=off"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,xino=on,xino=off"), FsFlags::empty());
        parse_expect_einval(
            Some("lowerdir=l,default_permissions,default_permissions"),
            FsFlags::empty(),
        );
        parse_expect_einval(Some("lowerdir=l,userxattr,userxattr"), FsFlags::empty());
    }

    #[ktest]
    fn parse_rejects_bare_valued_keys() {
        parse_expect_einval(Some("lowerdir"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,upperdir"), FsFlags::empty());
        parse_expect_einval(Some(",workdir"), FsFlags::empty());
        parse_expect_einval(Some(",uuid"), FsFlags::empty());
        parse_expect_einval(Some(",xino"), FsFlags::empty());
        parse_expect_einval(Some("foo=bar"), FsFlags::empty());
        parse_expect_einval(Some("foo"), FsFlags::empty());
        parse_expect_einval(Some("=value"), FsFlags::empty());
        parse_expect_einval(Some("LOWERDIR=l"), FsFlags::empty());
    }

    #[ktest]
    fn parse_valueless_keys() {
        let options = parse_expect_ok("lowerdir=l,default_permissions", FsFlags::empty());
        assert!(options.is_default_permissions);
        let options = parse_expect_ok("lowerdir=l,userxattr", FsFlags::empty());
        assert!(options.is_userxattr);
        let options = parse_expect_ok("lowerdir=l,default_permissions,userxattr", FsFlags::empty());
        assert!(options.is_default_permissions);
        assert!(options.is_userxattr);
        parse_expect_einval(Some("default_permissions=x"), FsFlags::empty());
        parse_expect_einval(Some("userxattr=1"), FsFlags::empty());
        parse_expect_einval(Some("userxattr="), FsFlags::empty());
    }

    #[ktest]
    fn parse_uuid_and_xino_values() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,uuid=off", FsFlags::empty()).uuid_mode,
            Some(UuidMode::Off)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,uuid=null", FsFlags::empty()).uuid_mode,
            Some(UuidMode::Null)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,uuid=on", FsFlags::empty()).uuid_mode,
            Some(UuidMode::On)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,uuid=auto", FsFlags::empty()).uuid_mode,
            Some(UuidMode::Auto)
        );
        parse_expect_einval(Some("uuid="), FsFlags::empty());
        parse_expect_einval(Some("uuid=ON"), FsFlags::empty());
        parse_expect_einval(Some("uuid=yes"), FsFlags::empty());
        assert_eq!(
            parse_expect_ok("lowerdir=l,xino=off", FsFlags::empty()).xino_mode,
            Some(XinoMode::Off)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,xino=auto", FsFlags::empty()).xino_mode,
            Some(XinoMode::Auto)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,xino=on", FsFlags::empty()).xino_mode,
            Some(XinoMode::On)
        );
        parse_expect_einval(Some("xino="), FsFlags::empty());
        parse_expect_einval(Some("xino=ON"), FsFlags::empty());
        parse_expect_einval(Some("xino=1"), FsFlags::empty());
        parse_expect_einval(Some("xino=on "), FsFlags::empty());
    }

    #[ktest]
    fn parse_literal_value_semantics() {
        let options = parse_expect_ok("lowerdir=a=b", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["a=b"]);
        let options = parse_expect_ok("lowerdir=\"a\"", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["\"a\""]);
        let options = parse_expect_ok("lowerdir=a b", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["a b"]);
        let options = parse_expect_ok("lowerdir= a", FsFlags::empty());
        assert_eq!(options.lower_dirs, [" a"]);
        parse_expect_einval(Some("lowerdir=l,uuid=on "), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=a,b"), FsFlags::empty());
        let options = parse_expect_ok(
            ",,lowerdir=l,,userxattr,,default_permissions,,",
            FsFlags::empty(),
        );
        assert!(options.is_userxattr);
        assert!(options.is_default_permissions);
    }

    #[ktest]
    fn parse_redirect_dir_domain() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,redirect_dir=on", FsFlags::empty()).redirect_dir,
            Some(RedirectDirMode::On)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,redirect_dir=follow", FsFlags::empty()).redirect_dir,
            Some(RedirectDirMode::Follow)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,redirect_dir=nofollow", FsFlags::empty()).redirect_dir,
            Some(RedirectDirMode::NoFollow)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,redirect_dir=off", FsFlags::empty()).redirect_dir,
            Some(RedirectDirMode::Off)
        );
        parse_expect_einval(Some("redirect_dir="), FsFlags::empty());
        parse_expect_einval(Some("redirect_dir=ON"), FsFlags::empty());
        parse_expect_einval(Some("redirect_dir=yes"), FsFlags::empty());
        parse_expect_einval(Some("redirect_dir"), FsFlags::empty());
        parse_expect_einval(Some("redirect_dir=on,redirect_dir=off"), FsFlags::empty());
    }

    #[ktest]
    fn parse_bool_option_domains() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,index=on", FsFlags::empty()).index,
            Some(true)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,index=off", FsFlags::empty()).index,
            Some(false)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,nfs_export=on", FsFlags::empty()).nfs_export,
            Some(true)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,nfs_export=off", FsFlags::empty()).nfs_export,
            Some(false)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,metacopy=on", FsFlags::empty()).metacopy,
            Some(true)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,metacopy=off", FsFlags::empty()).metacopy,
            Some(false)
        );
        parse_expect_einval(Some("index=1"), FsFlags::empty());
        parse_expect_einval(Some("index=ON"), FsFlags::empty());
        parse_expect_einval(Some("index="), FsFlags::empty());
        parse_expect_einval(Some("index"), FsFlags::empty());
        parse_expect_einval(Some("index=on,index=off"), FsFlags::empty());
    }

    #[ktest]
    fn parse_verity_and_fsync_domains() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,verity=off", FsFlags::empty()).verity,
            Some(VerityMode::Off)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,verity=on", FsFlags::empty()).verity,
            Some(VerityMode::On)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,verity=require", FsFlags::empty()).verity,
            Some(VerityMode::Require)
        );
        parse_expect_einval(Some("verity=required"), FsFlags::empty());
        parse_expect_einval(Some("verity="), FsFlags::empty());
        parse_expect_einval(Some("verity"), FsFlags::empty());
        parse_expect_einval(Some("verity=on,verity=off"), FsFlags::empty());
        assert_eq!(
            parse_expect_ok("lowerdir=l,fsync=auto", FsFlags::empty()).fsync_mode,
            Some(FsyncMode::Auto)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,fsync=strict", FsFlags::empty()).fsync_mode,
            Some(FsyncMode::Strict)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,fsync=volatile", FsFlags::empty()).fsync_mode,
            Some(FsyncMode::Volatile)
        );
        parse_expect_einval(Some("fsync=0"), FsFlags::empty());
        parse_expect_einval(Some("fsync="), FsFlags::empty());
        parse_expect_einval(Some("fsync"), FsFlags::empty());
        parse_expect_einval(Some("fsync=auto,fsync=strict"), FsFlags::empty());
    }

    #[ktest]
    fn parse_volatile_alias() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,volatile", FsFlags::empty()).fsync_mode,
            Some(FsyncMode::Volatile)
        );
        parse_expect_einval(Some("volatile=1"), FsFlags::empty());
        parse_expect_einval(Some("volatile="), FsFlags::empty());
        parse_expect_einval(Some("volatile,volatile"), FsFlags::empty());
        parse_expect_einval(Some("volatile,fsync=auto"), FsFlags::empty());
        parse_expect_einval(Some("fsync=auto,volatile"), FsFlags::empty());
    }

    #[ktest]
    fn parse_override_creds_forms() {
        parse_expect_einval(Some("lowerdir=l,override_creds"), FsFlags::empty());
        parse_expect_einval(Some("override_creds=on"), FsFlags::empty());
        let options = parse_expect_ok("lowerdir=l,nooverride_creds", FsFlags::empty());
        assert_eq!(options.redirect_dir, None);
        assert_eq!(options.index, None);
        assert_eq!(options.nfs_export, None);
        assert_eq!(options.metacopy, None);
        assert_eq!(options.verity, None);
        assert_eq!(options.fsync_mode, None);
        parse_expect_einval(Some("nooverride_creds=x"), FsFlags::empty());
        parse_expect_einval(Some("nonooverride_creds"), FsFlags::empty());
        parse_expect_einval(Some("nooverride_creds,nooverride_creds"), FsFlags::empty());
    }

    #[ktest]
    fn parse_lowerdir_plus_append() {
        let options = parse_expect_ok("lowerdir+=/a,lowerdir+=/b", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["/a", "/b"]);
        let options = parse_expect_ok("lowerdir+=/a:b", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["/a:b"]);
        parse_expect_einval(Some("lowerdir=/l,lowerdir+=/a"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir+=/a,lowerdir=/l"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir+="), FsFlags::empty());
        parse_expect_einval(Some("lowerdir+=/a,b"), FsFlags::empty());
    }

    #[ktest]
    fn parse_datadir_plus_rejected() {
        parse_expect_einval(Some("lowerdir=l,datadir+=/d"), FsFlags::empty());
    }

    #[ktest]
    fn parse_new_key_conflicts() {
        parse_expect_einval(
            Some("lowerdir=l,userxattr,redirect_dir=on"),
            FsFlags::empty(),
        );
        parse_expect_einval(
            Some("lowerdir=l,userxattr,redirect_dir=follow"),
            FsFlags::empty(),
        );
        parse_expect_einval(
            Some("lowerdir=l,userxattr,redirect_dir=off"),
            FsFlags::empty(),
        );
        parse_expect_ok(
            "lowerdir=l,userxattr,redirect_dir=nofollow",
            FsFlags::empty(),
        );
        parse_expect_einval(Some("lowerdir=l,userxattr,metacopy=on"), FsFlags::empty());
        parse_expect_ok("lowerdir=l,userxattr,metacopy=off", FsFlags::empty());
        parse_expect_einval(
            Some("lowerdir=l,metacopy=on,redirect_dir=nofollow"),
            FsFlags::empty(),
        );
        parse_expect_einval(
            Some("lowerdir=l,metacopy=on,redirect_dir=off"),
            FsFlags::empty(),
        );
        parse_expect_einval(
            Some("lowerdir=l,metacopy=on,redirect_dir=follow"),
            FsFlags::empty(),
        );
        parse_expect_ok("lowerdir=l,metacopy=on,redirect_dir=on", FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,nfs_export=on,index=off"), FsFlags::empty());
        parse_expect_einval(
            Some("lowerdir=l,nfs_export=on,metacopy=on"),
            FsFlags::empty(),
        );
    }
}
