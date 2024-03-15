// SPDX-License-Identifier: MPL-2.0

#![no_std]

use core::ops::{Deref, DerefMut};

bitflags::bitflags! {
    /// Value-based access rights.
    ///
    /// These access rights are provided to cover a wide range of use cases.
    /// The access rights' semantics and how they would restrict the behaviors
    /// of a capability are decided by the capability's designer.
    /// Here, we give some sensible semantics for each access right.
    pub struct Rights: u32 {
        /// Allows duplicating a capability.
        const DUP    = 1 << 0;
        /// Allows reading data from a data source (files, VM objects, etc.) or
        /// creating readable memory mappings.
        const READ     = 1 << 1;
        /// Allows writing data to a data sink (files, VM objects, etc.) or
        /// creating writable memory mappings.
        const WRITE    = 1 << 2;
        /// Allows creating executable memory mappings.
        const EXEC    = 1 << 3;
        /// Allows sending notifications or signals.
        const SIGNAL   = 1 << 7;
    }
}

typeflags::typeflags! {
    /// Type-based access rights.
    ///
    /// Similar to value-based access rights (`Rights`), but represented in
    /// types.
    pub trait TRights: u32 {
        /// Allows duplicating a capability.
        pub struct Dup     = Rights::DUP.bits;
        /// Allows reading data from a data source (files, VM objects, etc.) or
        /// creating readable memory mappings.
        pub struct Read     = Rights::READ.bits;
        /// Allows writing data to a data sink (files, VM objects, etc.) or
        /// creating writable memory mappings.
        pub struct Write    = Rights::WRITE.bits;
        /// Allows creating executable memory mappings.
        pub struct Exec     = Rights::EXEC.bits;
        /// Allows sending notifications or signals.
        pub struct Signal   = Rights::SIGNAL.bits;
    }
}

/// The full set of access rights.
pub type Full = TRightSet<TRights![Dup, Read, Write, Exec, Signal]>;
pub type ReadOp = TRights![Read];
pub type WriteOp = TRights![Write];
pub type FullOp = TRights![Read, Write, Dup];

/// Wrapper for TRights, used to bypass an error message from the Rust compiler,
/// the relevant issue is: <https://github.com/rust-lang/rfcs/issues/2758>
///
/// Example:
///
/// ```rust
/// use aster_rights::{Rights, TRights, TRightSet};
///
/// pub struct Vmo<R=Rights>(R);
///
/// impl<R:TRights> Vmo<TRightSet<R>>{
///     //...
/// }
///
/// impl Vmo<Rights>{
///     //...
/// }
///
/// ```
///
#[derive(Clone, Copy)]
pub struct TRightSet<T>(pub T);

impl<T> Deref for TRightSet<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for TRightSet<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
