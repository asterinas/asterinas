// SPDX-License-Identifier: MPL-2.0

use aster_systree::SysPerms;

use crate::prelude::*;

bitflags! {
    pub struct InodeMode: u16 {
        /// set-user-ID
        const S_ISUID = 0o4000;
        /// set-group-ID
        const S_ISGID = 0o2000;
        /// sticky bit
        const S_ISVTX = 0o1000;
        /// read by owner
        const S_IRUSR = 0o0400;
        /// write by owner
        const S_IWUSR = 0o0200;
        /// execute/search by owner
        const S_IXUSR = 0o0100;
        /// read by group
        const S_IRGRP = 0o0040;
        /// write by group
        const S_IWGRP = 0o0020;
        /// execute/search by group
        const S_IXGRP = 0o0010;
        /// read by others
        const S_IROTH = 0o0004;
        /// write by others
        const S_IWOTH = 0o0002;
        /// execute/search by others
        const S_IXOTH = 0o0001;
    }
}

impl InodeMode {
    pub fn is_owner_readable(&self) -> bool {
        self.contains(Self::S_IRUSR)
    }

    pub fn is_owner_writable(&self) -> bool {
        self.contains(Self::S_IWUSR)
    }

    pub fn is_owner_executable(&self) -> bool {
        self.contains(Self::S_IXUSR)
    }

    pub fn is_group_readable(&self) -> bool {
        self.contains(Self::S_IRGRP)
    }

    pub fn is_group_writable(&self) -> bool {
        self.contains(Self::S_IWGRP)
    }

    pub fn is_group_executable(&self) -> bool {
        self.contains(Self::S_IXGRP)
    }

    pub fn is_other_readable(&self) -> bool {
        self.contains(Self::S_IROTH)
    }

    pub fn is_other_writable(&self) -> bool {
        self.contains(Self::S_IWOTH)
    }

    pub fn is_other_executable(&self) -> bool {
        self.contains(Self::S_IXOTH)
    }

    #[expect(dead_code)]
    pub fn has_sticky_bit(&self) -> bool {
        self.contains(Self::S_ISVTX)
    }

    pub fn has_set_uid(&self) -> bool {
        self.contains(Self::S_ISUID)
    }

    pub fn has_set_gid(&self) -> bool {
        self.contains(Self::S_ISGID)
    }
}

impl From<SysPerms> for InodeMode {
    fn from(value: SysPerms) -> Self {
        InodeMode::from_bits_truncate(value.bits())
    }
}

/// Changes an inode mode.
///
/// # Syntax
///
/// If you are familiar with the `chmod` tool, the syntax of the `chmod` macro
/// should appear natural and self-explaining.
///
/// The `chmod` macro takes two or more arguments. The first argument is an
/// initial value of the inode mode. The rest arguments are one or more
/// symbolic mode, each of which dictates how the current value of the inode
/// mode shall be changed.
///
/// The format of a symbolic mode is as follows:
///
/// ```
/// [ugoa][+-=][perms]
/// ```
///
/// where `[perms]` is either zero or more letters from `rwx`.
///
/// The user part, `[ugoa]`,
/// controls the target users whose access to the file will be changed:
/// * `u`: the user who owns it;
/// * `g`: other users in the file owner's group;
/// * `o`: other users not in the file owner's group;
/// * `a`: all users.
///
/// The operator part, `[+-=]`,
/// indicates how the file permission bits are changed:
/// * `+`: `[perms]` are added to the target users' permissions;
/// * `-`: `[perms]` are removed from the target users' permissions;
/// * `=`: `[perms]` are set to the target users' permissions.
///
/// The permission part, `[perms]`, is a combination of `rwx`.
/// * `r`: the read permission;
/// * `w`: the write permission;
/// * `x`: the execute/search permission.
///
/// # Examples
///
/// ```
/// // Add the read and write permissions to everyone
/// let mode0 = mkmod!(a+rw);
/// assert!(mode0.bits() == 0o666);
///
/// // Add the execute permissions to the owner user
/// let mode1 = chmod!(mode0, u+x);
/// assert!(mode1.bits() == 0o766);
///
/// // Combine the above two steps into one invocation
/// let mode2 = mkmod!(a+rw, u+x);
/// assert!(mode2.bits() == 0o766);
/// ```
macro_rules! chmod {
    ($mode:expr) => { $mode };

    ($mode:expr, $who:ident + $perms:ident $(, $($rest:tt)*)?) => {
        $crate::fs::utils::chmod!(@apply $mode, $who, '+', $perms $(, $($rest)*)?)
    };
    ($mode:expr, $who:ident - $perms:ident $(, $($rest:tt)*)?) => {
        $crate::fs::utils::chmod!(@apply $mode, $who, '-', $perms $(, $($rest)*)?)
    };
    ($mode:expr, $who:ident = $perms:ident $(, $($rest:tt)*)?) => {
        $crate::fs::utils::chmod!(@apply $mode, $who, '=', $perms $(, $($rest)*)?)
    };
    ($mode:expr, $who:ident = $(, $($rest:tt)*)?) => {
        $crate::fs::utils::chmod!(@apply $mode, $who, '=', none $(, $($rest)*)?)
    };

    (@apply $mode:expr, $who:ident, $op:expr, $perms:ident $(, $($rest:tt)*)?) => {{
        let mask = $crate::fs::utils::who_and_perms_to_mask!($who, $perms);
        let new_mode = match $op {
            '+' => $mode | mask,
            '-' => $mode & !mask,
            '=' => ($mode & !$crate::fs::utils::who_and_perms_to_mask!($who, rwx)) | mask,
            _ => unreachable!(),
        };
        $crate::fs::utils::chmod!(new_mode $(, $($rest)*)?)
    }};
}

/// Makes an inode mode.
///
/// `mkmod` is equivalent to `chmod` with the first argument set to
/// `InodeMode::empty()`. See [`chmod`] for details.
macro_rules! mkmod {
    ($($args:tt)*) => {
        $crate::fs::utils::chmod!($crate::fs::utils::InodeMode::empty(), $($args)*)
    };
}

macro_rules! who_and_perms_to_mask {
    ($who:ident, $perms:ident) => {
        $crate::fs::utils::InodeMode::from_bits_truncate(
            $crate::fs::utils::who_to_mask!($who) & $crate::fs::utils::perms_to_mask!($perms),
        )
    };
}

macro_rules! who_to_mask {
    (u) => {
        0o700
    };
    (g) => {
        0o070
    };
    (o) => {
        0o007
    };
    (ug) => {
        0o770
    };
    (uo) => {
        0o707
    };
    (go) => {
        0o077
    };
    (a) => {
        0o777
    };
    (ugo) => {
        0o777
    };
}

macro_rules! perms_to_mask {
    (none) => {
        0
    };
    (r) => {
        0o444
    };
    (w) => {
        0o222
    };
    (x) => {
        0o111
    };
    (rw) => {
        0o666
    };
    (rx) => {
        0o555
    };
    (wx) => {
        0o333
    };
    (rwx) => {
        0o777
    };
}

pub(crate) use chmod;
pub(crate) use mkmod;
pub(crate) use perms_to_mask;
pub(crate) use who_and_perms_to_mask;
pub(crate) use who_to_mask;

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    #[ktest]
    fn mkmod_and_chmod() {
        let mode0 = mkmod!(a+rw);
        assert!(mode0.bits() == 0o666);

        let mode1 = chmod!(mode0, u+x);
        assert!(mode1.bits() == 0o766);

        let mode2 = mkmod!(a+rw, u+x);
        assert!(mode2.bits() == 0o766);

        let mode3 = chmod!(mode2, ug-wx);
        assert!(mode3.bits() == 0o446);

        let mode4 = chmod!(mode3, o=rx);
        assert!(mode4.bits() == 0o445);

        let mode5 = chmod!(mode4, u=, g=, o=);
        assert!(mode5.bits() == 0o000);
    }
}
