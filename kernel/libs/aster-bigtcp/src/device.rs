// SPDX-License-Identifier: MPL-2.0

pub use smoltcp::phy::{
    Checksum, ChecksumCapabilities, Device, DeviceCapabilities, Loopback, Medium, RxToken, TxToken,
};

/// A trait that allows to obtain a mutable reference of [`Device`].
///
/// A [`Device`] is usually protected by a lock (e.g., a spin lock or a mutex), and it may be
/// stored behind a shared type (e.g., an `Arc`). This property abstracts this fact by providing a
/// method that the caller can use to get the mutable reference without worrying about how the
/// reference is obtained.
pub trait WithDevice: Send + Sync {
    type Device: Device + ?Sized;

    /// Calls the closure with a mutable reference of [`Device`].
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Self::Device) -> R;
}
