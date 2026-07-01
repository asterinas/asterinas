// SPDX-License-Identifier: MPL-2.0

use aster_drm::DrmFeatures;

use crate::{
    device::drm::{DrmMinorType, file::DrmFile, has_current_sys_admin},
    prelude::*,
};

pub(super) mod defines;
pub(super) mod types;
pub(super) use defines::*;
pub(super) use types::*;

bitflags::bitflags! {
    pub(super) struct DrmIoctlFlags: u32 {
        const DEFAULT          = 0;
        const AUTH         = 1 << 0; // authenticated or master
        const MASTER       = 1 << 1; // requires current file is master
        const ROOT_ONLY    = 1 << 2; // requires CAP_SYS_ADMIN
        const RENDER_ALLOW = 1 << 3; // allowed on render node
        const MODESET      = 1 << 4; // requires a modeset-capable DRM driver
        const ATOMIC       = 1 << 5; // requires driver atomic modesetting support
        const GEM          = 1 << 6; // requires GEM memory-management support
        const SYNCOBJ      = 1 << 7; // requires sync object support
        const SYNCOBJ_TIMELINE = 1 << 8; // requires timeline sync object support
        const CURSOR_HOTSPOT   = 1 << 9; // requires virtualized cursor hotspot support
    }
}

pub(super) trait DrmIoctlFlagsInfo {
    const REQUIRED_FLAGS: DrmIoctlFlags;
}

fn check_required_driver_features(required_flags: DrmIoctlFlags, file: &DrmFile) -> Result<()> {
    let required_features = [
        (DrmIoctlFlags::MODESET, DrmFeatures::MODESET),
        (DrmIoctlFlags::ATOMIC, DrmFeatures::ATOMIC),
        (DrmIoctlFlags::GEM, DrmFeatures::GEM),
        (DrmIoctlFlags::SYNCOBJ, DrmFeatures::SYNCOBJ),
        (
            DrmIoctlFlags::SYNCOBJ_TIMELINE,
            DrmFeatures::SYNCOBJ_TIMELINE,
        ),
        (DrmIoctlFlags::CURSOR_HOTSPOT, DrmFeatures::CURSOR_HOTSPOT),
    ];

    for (required_flag, driver_feature) in required_features {
        if required_flags.contains(required_flag) && !file.has_feature(driver_feature) {
            return_errno!(Errno::EOPNOTSUPP);
        }
    }

    Ok(())
}

pub(super) fn check_drm_ioctl_flags<T: DrmIoctlFlagsInfo>(file: &DrmFile) -> Result<()> {
    let required_flags = T::REQUIRED_FLAGS;
    let minor_type = file.minor_type();

    match minor_type {
        DrmMinorType::Primary => {
            if required_flags.contains(DrmIoctlFlags::AUTH) && !file.is_authenticated() {
                return_errno!(Errno::EACCES);
            }
        }
        DrmMinorType::Render => {
            if !required_flags.contains(DrmIoctlFlags::RENDER_ALLOW) {
                return_errno!(Errno::EACCES);
            }
        }
        _ => {
            // TODO: control/accel policy
            return_errno!(Errno::EACCES);
        }
    }

    if required_flags.contains(DrmIoctlFlags::ROOT_ONLY) && !has_current_sys_admin() {
        return_errno!(Errno::EACCES);
    }
    if required_flags.contains(DrmIoctlFlags::MASTER) && !file.is_master() {
        return_errno!(Errno::EACCES);
    }

    check_required_driver_features(required_flags, file)?;

    Ok(())
}

macro_rules! drm_ioc {
    ($name:ident, $linux_name:ident, $nr:expr, $data:ty, $perm:expr) => {
        pub type $name = $crate::util::ioctl::ioc!($linux_name, b'd', $nr, $data);

        impl $crate::device::drm::ioctl::DrmIoctlFlagsInfo for $name {
            const REQUIRED_FLAGS: $crate::device::drm::ioctl::DrmIoctlFlags = $perm;
        }
    };
}
pub(super) use drm_ioc;

/// Dispatches a DRM ioctl after validating generic file permissions and
/// driver feature requirements.
///
/// Unlike Linux's table-based dispatch, this helper performs the common
/// permission gate before entering the matched ioctl handler and also
/// enforces Asterinas-specific driver feature checks encoded in
/// `DrmIoctlFlags` such as `MODESET` and `ATOMIC`.
macro_rules! drm_dispatch {
    ($file:expr, match $raw:ident {}) => {
        ()
    };

    ($file:expr, match $raw:ident { _ => $arm:expr $(,)? }) => {
        $arm
    };

    ($file:expr, match $raw:ident {
        $ty0:ty $(| $ty1:ty)* => $arm:block $(,)?
        $($rest:tt)*
    }) => {
        if <$ty0>::try_from_raw($raw).is_some() {
            $crate::device::drm::ioctl::check_drm_ioctl_flags::<$ty0>($file)?;
            $arm
        } $( else if <$ty1>::try_from_raw($raw).is_some() {
            $crate::device::drm::ioctl::check_drm_ioctl_flags::<$ty1>($file)?;
            $arm
        } )* else {
            $crate::device::drm::ioctl::drm_dispatch!($file, match $raw { $($rest)* })
        }
    };

    ($file:expr, match $raw:ident {
        $bind:ident @ $ty:ty => $arm:block $(,)?
        $($rest:tt)*
    }) => {
        if let Some($bind) = <$ty>::try_from_raw($raw) {
            $crate::device::drm::ioctl::check_drm_ioctl_flags::<$ty>($file)?;
            $arm
        } else {
            $crate::device::drm::ioctl::drm_dispatch!($file, match $raw { $($rest)* })
        }
    };

    ($file:expr, match $raw:ident {
        $ty0:ty $(| $ty1:ty)* => $arm:expr,
        $($rest:tt)*
    }) => {
        if <$ty0>::try_from_raw($raw).is_some() {
            $crate::device::drm::ioctl::check_drm_ioctl_flags::<$ty0>($file)?;
            $arm
        } $( else if <$ty1>::try_from_raw($raw).is_some() {
            $crate::device::drm::ioctl::check_drm_ioctl_flags::<$ty1>($file)?;
            $arm
        } )* else {
            $crate::device::drm::ioctl::drm_dispatch!($file, match $raw { $($rest)* })
        }
    };

    ($file:expr, match $raw:ident {
        $bind:ident @ $ty:ty => $arm:expr,
        $($rest:tt)*
    }) => {
        if let Some($bind) = <$ty>::try_from_raw($raw) {
            $crate::device::drm::ioctl::check_drm_ioctl_flags::<$ty>($file)?;
            $arm
        } else {
            $crate::device::drm::ioctl::drm_dispatch!($file, match $raw { $($rest)* })
        }
    };
}
pub(super) use drm_dispatch;

macro_rules! dispatch_drm_ioctl {
    ($file:expr, $($tt:tt)*) => {
        $crate::device::drm::ioctl::drm_dispatch!($file, $($tt)*)
    };
}
pub(super) use dispatch_drm_ioctl;
