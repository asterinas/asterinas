// SPDX-License-Identifier: MPL-2.0

/// Extension trait that provides convenient byte conversion methods for types implementing `bytemuck::Pod`.
///
/// # Motivation
///
/// The `bytemuck::Pod` trait is a marker trait that only indicates a type is safely transmutable.
/// However, `Pod` itself doesn't provide methods to convert to/from bytes. Users must rely on
/// `bytemuck::bytes_of()` and `bytemuck::from_bytes()` functions, which is verbose and repetitive.
///
/// This trait bridges that gap by providing convenient methods on any type that implements `Pod`,
/// allowing idiomatic syntax like `my_value.as_bytes()` instead of `bytemuck::bytes_of(&my_value)`.
pub trait PodExtension: bytemuck::Pod {
    /// Returns an immutable view of this value as a byte slice.
    ///
    /// This is a convenience wrapper around [`bytemuck::bytes_of()`].
    fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    /// Returns a mutable view of this value as a byte slice.
    ///
    /// This is a convenience wrapper around [`bytemuck::bytes_of_mut()`].
    fn as_bytes_mut(&mut self) -> &mut [u8] {
        bytemuck::bytes_of_mut(self)
    }

    /// Constructs a value of this type from a byte slice.
    ///
    /// This is a convenience wrapper around [`bytemuck::from_bytes()`].
    fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}

/// Blanket implementation of `PodExtension` for all types implementing `bytemuck::Pod`.
impl<T: bytemuck::Pod> PodExtension for T {}
