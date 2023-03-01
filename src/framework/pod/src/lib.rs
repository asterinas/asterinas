#![no_std]

use core::mem::MaybeUninit;

/// A marker trait for plain old data (POD).
///
/// A POD type `T:Pod` supports converting to and from arbitrary
/// `mem::size_of::<T>()` bytes _safely_.
/// For example, simple primitive types like `u8` and `i16`
/// are POD types. But perhaps surprisingly, `bool` is not POD
/// because Rust compiler makes implicit assumption that
/// a byte of `bool` has a value of either `0` or `1`.
/// Interpreting a byte of value `3` has a `bool` value has
/// undefined behavior.
///
/// # Safety
///
/// Marking a non-POD type as POD may cause undefined behaviors.
pub unsafe trait Pod: Copy + Sized {
    /// Creates a new instance of Pod type that is filled with zeroes.
    fn new_zeroed() -> Self {
        // SAFETY. An all-zero value of `T: Pod` is always valid.
        unsafe { core::mem::zeroed() }
    }

    /// Creates a new instance of Pod type with uninitialized content.
    fn new_uninit() -> Self {
        // SAFETY. A value of `T: Pod` can have arbitrary bits.
        #[allow(clippy::uninit_assumed_init)]
        unsafe {
            MaybeUninit::uninit().assume_init()
        }
    }

    /// Creates a new instance from the given bytes.
    fn from_bytes(bytes: &[u8]) -> Self {
        let mut new_self = Self::new_uninit();
        let copy_len = new_self.as_bytes().len();
        new_self.as_bytes_mut().copy_from_slice(&bytes[..copy_len]);
        new_self
    }

    /// As a slice of bytes.
    fn as_bytes(&self) -> &[u8] {
        let ptr = self as *const Self as *const u8;
        let len = core::mem::size_of::<Self>();
        unsafe { core::slice::from_raw_parts(ptr, len) }
    }

    /// As a mutable slice of bytes.
    fn as_bytes_mut(&mut self) -> &mut [u8] {
        let ptr = self as *mut Self as *mut u8;
        let len = core::mem::size_of::<Self>();
        unsafe { core::slice::from_raw_parts_mut(ptr, len) }
    }
}

macro_rules! impl_pod_for {
    ($($pod_ty:ty),*) => {
        $(unsafe impl Pod for $pod_ty {})*
    };
}
// impl Pod for primitive types
impl_pod_for!(u8, u16, u32, u64, i8, i16, i32, i64, isize, usize);
// impl Pod for array
unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}

#[cfg(feature = "derive")]
pub use pod_derive::*;
