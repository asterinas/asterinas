// SPDX-License-Identifier: MPL-2.0

//! Mount option parsing for overlayfs.
//!
//! This module validates the mount option string into a
//! [`MountOptions`] construction input. The recognized keys are the path
//! keys `lowerdir`, `lowerdir+`, `upperdir`, and `workdir`, the mode keys
//! `uuid` and `xino`, the raw-intent keys `redirect_dir`, `index`,
//! `nfs_export`, `metacopy`, `verity`, and `fsync`, and the valueless keys
//! `default_permissions`, `userxattr`, `volatile`, and `nooverride_creds`.
//!
//! Unknown keys fail with `EINVAL` before any layer state is created. A
//! raw-intent key records an explicitly requested feature that is not
//! implemented: the request is accepted, and the verify phase warns once at
//! mount time that the mount proceeds without that feature.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v7.0/source/Documentation/filesystems/overlayfs.rst#L350-L364>
//!   (Linux stacks colon-separated lowerdirs with the first entry topmost)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/params.c>
//!   (upstream option parse and `ovl_fs_params_verify`: key tables, value domains, the `volatile` alias for `fsync=volatile`, and the cross-key conflict rules)

use super::super::policy::{UuidMode, XinoMode};
use crate::{
    fs::vfs::{file_system::FsFlags, xattr::XattrNamespace},
    prelude::*,
};

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

#[derive(Debug, Default)]
pub(super) struct MountOptions {
    /// The caller asked the overlay to run its own permission checks.
    pub(super) is_default_permissions: bool,
    /// The `lowerdir`/`lowerdir+` layer paths in merge order, topmost first.
    pub(super) lower_dirs: Vec<String>,
    /// The `upperdir` path; `None` on a mount that has no upper.
    pub(super) upper_dir: Option<String>,
    /// The `workdir` path; present exactly when `upper_dir` is.
    pub(super) work_dir: Option<String>,
    /// Whether the mount request carried `FsFlags::RDONLY`.
    pub(super) is_forced_read_only: bool,
    /// The `userxattr` key was requested; selects the `user.` private-record prefix.
    pub(super) is_userxattr: bool,
    /// The `uuid` key's mode; the mount default applies when the key is absent.
    pub(super) uuid_mode: Option<UuidMode>,
    /// The `xino` key's mode; the mount default applies when the key is absent.
    pub(super) xino_mode: Option<XinoMode>,
    redirect_dir: Option<RedirectDirMode>,
    index: Option<bool>,
    nfs_export: Option<bool>,
    metacopy: Option<bool>,
    verity: Option<VerityMode>,
    fsync_mode: Option<FsyncMode>,
    is_lowerdir_plus_seen: bool,
    is_nooverride_creds_seen: bool,
}

const LOWERDIR_REQUIRED_MESSAGE: &str = "the `lowerdir` mount option must be specified";
const DUPLICATE_KEY_MESSAGE: &str = "duplicate overlay mount option";
const LOWERDIR_MIXING_MESSAGE: &str =
    "the `lowerdir+` mount option cannot be combined with `lowerdir`";
const INVALID_VALUE_MESSAGE: &str = "invalid overlay mount option value";

impl MountOptions {
    pub(super) fn parse(args: Option<&str>, fs_flags: FsFlags) -> Result<Self> {
        let Some(args) = args else {
            return_errno_with_message!(Errno::EINVAL, LOWERDIR_REQUIRED_MESSAGE);
        };

        let mut options = Self {
            is_forced_read_only: fs_flags.contains(FsFlags::RDONLY),
            ..Default::default()
        };

        for entry in args.split(',') {
            if entry.is_empty() {
                continue;
            }
            let (key, value) = entry
                .split_once('=')
                .map_or((entry, None), |(key, value)| (key, Some(value)));
            options.set_entry(key, value)?;
        }

        if options.lower_dirs.is_empty() {
            return_errno_with_message!(Errno::EINVAL, LOWERDIR_REQUIRED_MESSAGE);
        }
        options.verify()?;
        Ok(options)
    }

    /// Returns the xattr namespace the mount's private records live under.
    pub(super) fn xattr_namespace(&self) -> XattrNamespace {
        if self.is_userxattr {
            XattrNamespace::User
        } else {
            XattrNamespace::Trusted
        }
    }

    /// Logs the one-shot mount-time degrade of one requested unimplemented feature.
    fn degrade(message: impl core::fmt::Display) {
        warn!("{message}");
    }

    fn set_entry(&mut self, key: &str, value: Option<&str>) -> Result<()> {
        match key {
            "lowerdir" => {
                let value = Self::require_value(value)?;
                if !self.lower_dirs.is_empty() {
                    if self.is_lowerdir_plus_seen {
                        return_errno_with_message!(Errno::EINVAL, LOWERDIR_MIXING_MESSAGE);
                    }
                    return_errno_with_message!(Errno::EINVAL, DUPLICATE_KEY_MESSAGE);
                }
                self.lower_dirs = value.split(':').map(str::to_string).collect();
                if self.lower_dirs.iter().any(|lower_dir| lower_dir.is_empty()) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the `lowerdir` value contains an empty layer path"
                    );
                }
            }
            "lowerdir+" => {
                let value = Self::require_value(value)?;
                if !self.lower_dirs.is_empty() && !self.is_lowerdir_plus_seen {
                    return_errno_with_message!(Errno::EINVAL, LOWERDIR_MIXING_MESSAGE);
                }
                self.is_lowerdir_plus_seen = true;
                self.lower_dirs.push(value.to_string());
            }
            "upperdir" => {
                Self::set_once(&mut self.upper_dir, Self::require_value(value)?.to_string())?
            }
            "workdir" => {
                Self::set_once(&mut self.work_dir, Self::require_value(value)?.to_string())?
            }
            "uuid" => Self::set_once(
                &mut self.uuid_mode,
                match Self::require_value(value)? {
                    "off" => UuidMode::Off,
                    "null" => UuidMode::Null,
                    "on" => UuidMode::On,
                    "auto" => UuidMode::Auto,
                    _ => return_errno_with_message!(Errno::EINVAL, INVALID_VALUE_MESSAGE),
                },
            )?,
            "xino" => Self::set_once(
                &mut self.xino_mode,
                match Self::require_value(value)? {
                    "off" => XinoMode::Off,
                    "auto" => XinoMode::Auto,
                    "on" => XinoMode::On,
                    _ => return_errno_with_message!(Errno::EINVAL, INVALID_VALUE_MESSAGE),
                },
            )?,
            "redirect_dir" => Self::set_once(
                &mut self.redirect_dir,
                match Self::require_value(value)? {
                    "off" => RedirectDirMode::Off,
                    "follow" => RedirectDirMode::Follow,
                    "nofollow" => RedirectDirMode::NoFollow,
                    "on" => RedirectDirMode::On,
                    _ => return_errno_with_message!(Errno::EINVAL, INVALID_VALUE_MESSAGE),
                },
            )?,
            "index" => Self::set_once(
                &mut self.index,
                Self::parse_bool(Self::require_value(value)?)?,
            )?,
            "nfs_export" => Self::set_once(
                &mut self.nfs_export,
                Self::parse_bool(Self::require_value(value)?)?,
            )?,
            "metacopy" => Self::set_once(
                &mut self.metacopy,
                Self::parse_bool(Self::require_value(value)?)?,
            )?,
            "verity" => Self::set_once(
                &mut self.verity,
                match Self::require_value(value)? {
                    "off" => VerityMode::Off,
                    "on" => VerityMode::On,
                    "require" => VerityMode::Require,
                    _ => return_errno_with_message!(Errno::EINVAL, INVALID_VALUE_MESSAGE),
                },
            )?,
            "fsync" => Self::set_once(
                &mut self.fsync_mode,
                match Self::require_value(value)? {
                    "volatile" => FsyncMode::Volatile,
                    "auto" => FsyncMode::Auto,
                    "strict" => FsyncMode::Strict,
                    _ => return_errno_with_message!(Errno::EINVAL, INVALID_VALUE_MESSAGE),
                },
            )?,
            "default_permissions" => {
                // The overlay performs no permission check of its own, so the key asks for the only behavior this mount has.
                Self::require_bare(value)?;
                Self::set_flag_once(&mut self.is_default_permissions)?;
            }
            "userxattr" => {
                Self::require_bare(value)?;
                Self::set_flag_once(&mut self.is_userxattr)?;
            }
            "volatile" => {
                Self::require_bare(value)?;
                Self::set_once(&mut self.fsync_mode, FsyncMode::Volatile)?;
            }
            "nooverride_creds" => {
                // `nooverride_creds` is a no-op: caller credentials are already checked.
                Self::require_bare(value)?;
                Self::set_flag_once(&mut self.is_nooverride_creds_seen)?;
            }
            _ => {
                return_errno_with_message!(Errno::EINVAL, "unknown overlay mount option");
            }
        }
        Ok(())
    }

    /// Returns the non-empty value the entry carried for a key that takes one.
    fn require_value(value: Option<&str>) -> Result<&str> {
        let Some(value) = value else {
            return_errno_with_message!(Errno::EINVAL, "the overlay mount option requires a value");
        };
        if value.is_empty() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the overlay mount option requires a non-empty value"
            );
        }
        Ok(value)
    }

    /// Rejects a value given to a key that takes none.
    fn require_bare(value: Option<&str>) -> Result<()> {
        if value.is_some() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the overlay mount option does not take a value"
            );
        }
        Ok(())
    }

    /// Parses the `on`/`off` value domain shared by the boolean keys.
    fn parse_bool(value: &str) -> Result<bool> {
        match value {
            "on" => Ok(true),
            "off" => Ok(false),
            _ => return_errno_with_message!(Errno::EINVAL, INVALID_VALUE_MESSAGE),
        }
    }

    /// Stores a key's parsed value, rejecting a second spelling of the key.
    fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<()> {
        if slot.is_some() {
            return_errno_with_message!(Errno::EINVAL, DUPLICATE_KEY_MESSAGE);
        }
        *slot = Some(value);
        Ok(())
    }

    /// Records a valueless key once.
    fn set_flag_once(flag: &mut bool) -> Result<()> {
        if *flag {
            return_errno_with_message!(Errno::EINVAL, DUPLICATE_KEY_MESSAGE);
        }
        *flag = true;
        Ok(())
    }

    fn verify(&self) -> Result<()> {
        if self.upper_dir.is_some() != self.work_dir.is_some() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the `workdir` mount option is required if and only if `upperdir` is specified"
            );
        }
        if self.is_userxattr
            && self
                .redirect_dir
                .is_some_and(|mode| mode != RedirectDirMode::NoFollow)
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "conflicting overlay mount options: `userxattr` requires `redirect_dir=nofollow`"
            );
        }
        if self.is_userxattr && self.metacopy == Some(true) {
            return_errno_with_message!(
                Errno::EINVAL,
                "conflicting overlay mount options: `userxattr` and `metacopy=on`"
            );
        }
        if self.metacopy == Some(true)
            && self
                .redirect_dir
                .is_some_and(|mode| mode != RedirectDirMode::On)
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "conflicting overlay mount options: `metacopy=on` requires `redirect_dir=on`"
            );
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

        if let Some(mode) = self
            .redirect_dir
            .filter(|mode| *mode != RedirectDirMode::NoFollow)
        {
            Self::degrade(format_args!(
                "`redirect_dir={}`: degrading to redirect_dir=nofollow; directory redirects are neither recorded nor followed",
                mode.option_value()
            ));
        }
        if self.index == Some(true) {
            Self::degrade("`index=on`: degrading to index=off; no inode index is maintained");
        }
        if self.nfs_export == Some(true) {
            Self::degrade(
                "`nfs_export=on`: degrading to nfs_export=off; no export file handles are encoded",
            );
        }
        if self.metacopy == Some(true) {
            Self::degrade("`metacopy=on`: degrading to metacopy=off; copy-up always copies data");
        }
        if let Some(mode) = self.verity.filter(|mode| *mode != VerityMode::Off) {
            Self::degrade(format_args!(
                "`verity={}`: degrading to verity=off; fs-verity digests are not enforced",
                mode.option_value()
            ));
        }
        match self.fsync_mode {
            Some(FsyncMode::Strict) => {
                Self::degrade(
                    "`fsync=strict`: metadata/directory copy-up is not explicitly synced",
                );
            }
            Some(FsyncMode::Volatile) => {
                Self::degrade(
                    "`fsync=volatile`: sync suppression, the volatile dirty marker, and sticky syncfs errors are not implemented; durability follows the underlying filesystem",
                );
            }
            Some(FsyncMode::Auto) | None => {}
        }
        if self.is_default_permissions {
            Self::degrade(
                "`default_permissions`: the overlay performs no real-side permission re-check, so the requested behavior is already the only one; the option is inert",
            );
        }
        Ok(())
    }
}

impl RedirectDirMode {
    /// Returns the `redirect_dir=` value this mode is spelled with.
    fn option_value(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Follow => "follow",
            Self::NoFollow => "nofollow",
            Self::On => "on",
        }
    }
}

impl VerityMode {
    /// Returns the `verity=` value this mode is spelled with.
    fn option_value(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Require => "require",
        }
    }
}

#[cfg(ktest)]
mod test {
    //! Unit tests for the pure [`MountOptions::parse`] contract.
    //!
    //! The assertions form a fixed case table over the parse surface: every
    //! accepted spelling with the fields it sets, and every rejected spelling
    //! with the `EINVAL` it raises.

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
