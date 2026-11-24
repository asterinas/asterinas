// SPDX-License-Identifier: MPL-2.0

//! Published mount policy state.
//!
//! The upper-filesystem capabilities are measured during mount construction
//! and published here as fixed state; the policy performs no probing itself.
//! The filesystem options are settled with the policy and rendered
//! from it on demand.

#![short_vis_path::add(overlayfs)]

use super::{
    capabilities::{UpperFilesystemCapabilities, WhiteoutCapability},
    inuse::Uuid,
    options::{MountOptions, UuidMode, XinoMode},
};
use crate::{
    fs::{fs_impls::overlayfs::inode::IdentityMode, vfs::xattr::XattrNamespace},
    prelude::*,
};

/// The per-filesystem state settled once during construction, including its filesystem options.
pub(in overlayfs) struct MountPolicy {
    /// Mutating permission checks fail with `EROFS` while set; covers read-only mounts and uppers.
    is_effective_read_only: bool,
    /// The effective overlay UUID published as the superblock fsid; `None` when none persists.
    uuid: Option<Uuid>,
    /// Upper capabilities probed after the claim; `None` for lower-only and read-only mounts.
    upper_capabilities: Option<UpperFilesystemCapabilities>,
    /// The xattr namespace for the overlay's own records; kept even for read-only mounts.
    xattr_namespace: XattrNamespace,
    /// The filesystem options this filesystem records and reports for `/proc`.
    fs_options: FsOptionsRecorder,
}

/// The filesystem options recorded at construction and rendered for `/proc` on demand.
struct FsOptionsRecorder {
    /// The `lowerdir`/`lowerdir+` layer paths in merge order, topmost first.
    lower_dirs: Vec<String>,
    /// Whether the request used `lowerdir+` rather than the joined `lowerdir` form.
    is_lowerdir_plus_seen: bool,
    /// The `upperdir` path as the request named it; `None` on a mount with no upper.
    upper_dir: Option<String>,
    /// The `workdir` path as the request named it; present exactly when `upper_dir` is.
    work_dir: Option<String>,
    /// Whether the caller asked the overlay to run its own permission checks.
    is_default_permissions: bool,
    /// The `xino` key's mode as requested; the mount default applies when the key is absent.
    xino_mode: Option<XinoMode>,
    /// The `uuid` mode the mount requested, with the option default applied.
    uuid_request: UuidMode,
}

impl MountPolicy {
    /// Assembles the settled policy from the parsed mount request and the construction results.
    pub(super) fn assemble(
        options: MountOptions,
        is_effective_read_only: bool,
        uuid: Option<Uuid>,
        upper_capabilities: Option<UpperFilesystemCapabilities>,
    ) -> Self {
        let uuid_request = options.requested_uuid_mode();
        let MountOptions {
            is_default_permissions,
            lower_dirs,
            upper_dir,
            work_dir,
            xattr_namespace,
            xino_mode,
            is_lowerdir_plus_seen,
            ..
        } = options;
        Self {
            is_effective_read_only,
            uuid,
            upper_capabilities,
            xattr_namespace,
            fs_options: FsOptionsRecorder {
                lower_dirs,
                is_lowerdir_plus_seen,
                upper_dir,
                work_dir,
                is_default_permissions,
                xino_mode,
                uuid_request,
            },
        }
    }

    /// Renders this mount's own `/proc` options segment.
    pub(in overlayfs) fn render_fs_options(&self, identity_mode: IdentityMode) -> String {
        self.fs_options.render(
            identity_mode,
            self.settled_uuid_mode(),
            self.xattr_namespace,
        )
    }

    /// Returns the `uuid` mode this mount reports.
    fn settled_uuid_mode(&self) -> UuidMode {
        if self.uuid.is_some() {
            return UuidMode::On;
        }
        match self.fs_options.uuid_request {
            UuidMode::Off => UuidMode::Off,
            UuidMode::Null => UuidMode::Null,
            // Linux `ovl_fs_params_verify` turns `uuid=on` without an upper into `uuid=null`.
            //
            // Reference:
            // <https://elixir.bootlin.com/linux/v7.2/source/fs/overlayfs/params.c#L901-L903>.
            UuidMode::On => UuidMode::Null,
            UuidMode::Auto if self.fs_options.upper_dir.is_some() => UuidMode::Null,
            UuidMode::Auto => UuidMode::Auto,
        }
    }

    /// Returns the `xino` mode this mount requested, with the option default applied.
    pub(super) fn xino_mode(&self) -> XinoMode {
        self.fs_options.xino_mode.unwrap_or_default()
    }

    pub(in overlayfs) fn is_effective_read_only(&self) -> bool {
        self.is_effective_read_only
    }

    pub(in overlayfs) fn xattr_namespace(&self) -> XattrNamespace {
        self.xattr_namespace
    }

    pub(in super::super) fn uuid(&self) -> Option<&Uuid> {
        self.uuid.as_ref()
    }

    fn upper_capabilities(&self) -> Option<&UpperFilesystemCapabilities> {
        self.upper_capabilities.as_ref()
    }

    /// Returns the whiteout form the upper can publish; `Unsupported` means no form.
    pub(in overlayfs) fn whiteout_capability(&self) -> WhiteoutCapability {
        self.upper_capabilities()
            .map_or(WhiteoutCapability::Unsupported, |capabilities| {
                capabilities.whiteout_capability()
            })
    }

    /// Returns whether the probed upper can store the overlay's private xattrs.
    pub(in overlayfs) fn can_store_private_xattr(&self) -> bool {
        self.upper_capabilities
            .is_some_and(|caps| caps.can_store_private_xattr())
    }

    pub(in overlayfs) fn can_express_whiteout(&self) -> bool {
        self.whiteout_capability() != WhiteoutCapability::Unsupported
    }
}

impl FsOptionsRecorder {
    /// Renders this mount's options into a fresh `/proc` segment string.
    fn render(
        &self,
        identity_mode: IdentityMode,
        uuid_mode: UuidMode,
        xattr_namespace: XattrNamespace,
    ) -> String {
        let mut rendered = String::new();
        if self.is_lowerdir_plus_seen {
            for lower_dir in self.lower_dirs.iter() {
                Self::push_kv_option(&mut rendered, "lowerdir+", Some(lower_dir.as_str()));
            }
        } else {
            let lower_dirs = self.lower_dirs.join(":");
            Self::push_kv_option(&mut rendered, "lowerdir", Some(lower_dirs.as_str()));
        }
        Self::push_kv_option(&mut rendered, "upperdir", self.upper_dir.as_deref());
        Self::push_kv_option(&mut rendered, "workdir", self.work_dir.as_deref());
        if self.is_default_permissions {
            Self::push_bare_option(&mut rendered, "default_permissions");
        }
        Self::push_kv_option(&mut rendered, "uuid", uuid_mode.to_str());
        Self::push_kv_option(&mut rendered, "xino", self.xino_option_value(identity_mode));
        if xattr_namespace == XattrNamespace::User {
            Self::push_bare_option(&mut rendered, "userxattr");
        }
        rendered
    }

    /// Appends `name=value` when `value` is `Some`, escaping the value and separating entries with `,`.
    fn push_kv_option(rendered: &mut String, name: &str, value: Option<&str>) {
        let Some(value) = value else {
            return;
        };
        if !rendered.is_empty() {
            rendered.push(',');
        }
        rendered.push_str(name);
        rendered.push('=');
        rendered.push_str(&Self::escape_mount_option_value(value));
    }

    /// Appends the valueless key `name`, separating entries with `,`.
    fn push_bare_option(rendered: &mut String, name: &str) {
        if !rendered.is_empty() {
            rendered.push(',');
        }
        rendered.push_str(name);
    }

    /// Escapes a value so it stays one `/proc` option entry.
    ///
    /// Because the options field separates entries with `,` and the line with
    /// spaces, `,`, space, tab, newline, and backslash are octal-escaped as in
    /// Linux `seq_show_option`.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/v7.2/source/include/linux/seq_file.h#L264-L273>.
    fn escape_mount_option_value(value: &str) -> String {
        let mut escaped = String::with_capacity(value.len());
        for character in value.chars() {
            match character {
                ',' => escaped.push_str("\\054"),
                ' ' => escaped.push_str("\\040"),
                '\t' => escaped.push_str("\\011"),
                '\n' => escaped.push_str("\\012"),
                '\\' => escaped.push_str("\\134"),
                _ => escaped.push(character),
            }
        }
        escaped
    }

    /// Returns the `xino=` value the settled projection reports, or `None` when suppressed.
    fn xino_option_value(&self, identity_mode: IdentityMode) -> Option<&'static str> {
        match identity_mode {
            // Every layer is on one filesystem, so there is no encoding to report.
            IdentityMode::SameFs => None,
            IdentityMode::Xino { .. } => match self.xino_mode.unwrap_or_default() {
                XinoMode::On => Some("on"),
                // `Auto` is the default, so it is suppressed.
                XinoMode::Auto => None,
                XinoMode::Off => Some("off"),
            },
            // The fallback's effective value is `off`.
            IdentityMode::Fallback => Some("off"),
        }
    }
}
