// SPDX-License-Identifier: MPL-2.0

use core::ops::{Deref, DerefMut};

use zerocopy::{FromBytes, FromZeros, Immutable, KnownLayout};

/// A byte array with custom alignment.
///
/// `AlignedBytes<T, N>` is a newtype wrapper around `[u8; N]` that ensures the byte array
/// has the same alignment as type `T`. This is useful when you need to ensure that a byte
/// buffer has specific alignment requirements compatible with a particular type.
///
/// # Type Parameters
///
/// * `T` - The type that determines the alignment. Must implement `FromZeros`.
/// * `N` - The size of the byte array in bytes.
///
/// # Examples
///
/// Creating an 8-byte array with 64-bit alignment:
///
/// ```
/// use ostd_pod::*;
///
/// let mut aligned: AlignedBytes<u64, 8> = AlignedBytes::new_zeroed();
/// let bytes = aligned.as_mut_bytes();
/// bytes[0] = 42;
/// assert_eq!(aligned.as_bytes()[0], 42);
/// ```
///
/// Creating a 16-byte array with struct alignment:
///
/// ```
/// use ostd_pod::{AlignedBytes, derive, FromZeros, IntoBytes};
///
/// #[repr(C)]
/// #[derive(Pod, Clone, Copy)]
/// struct Data {
///     x: u32,
///     y: u32,
/// }
///
/// let aligned: AlignedBytes<Data, 16> = AlignedBytes::new_zeroed();
/// // The byte array is now aligned according to `Data`'s alignment requirement
/// assert_eq!(aligned.as_bytes().len(), 16);
/// assert_eq!(align_of::<Data>(), align_of_val(&aligned));
/// ```
///
/// # Alignment Guarantees
///
/// The byte array will have the same alignment as `T`. This means:
/// - `AlignedBytes<u32, N>` is aligned to 4 bytes
/// - `AlignedBytes<u64, N>` is aligned to 8 bytes
/// - `AlignedBytes<u128, N>` is aligned to 16 bytes
#[repr(C)]
#[derive(Clone, Copy, FromBytes, KnownLayout, Immutable)]
pub struct AlignedBytes<T, const N: usize> {
    _alignment: [T; 0],
    array: [u8; N],
}

impl<T: FromZeros, const N: usize> Default for AlignedBytes<T, N> {
    /// Creates a default `AlignedBytes` with all bytes initialized to zero.
    fn default() -> Self {
        Self::new_zeroed()
    }
}

impl<T, const N: usize> Deref for AlignedBytes<T, N> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.array
    }
}

impl<T, const N: usize> DerefMut for AlignedBytes<T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.array
    }
}
