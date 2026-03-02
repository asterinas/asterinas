// SPDX-License-Identifier: MPL-2.0

//! Aligned array helpers for Pod types.
//!
//! This module provides type-level utilities
//! for creating arrays with specific alignment requirements.
//! It's primarily used internally to support Pod unions
//! that need to maintain precise memory layouts with guaranteed alignment.

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// A transparent wrapper around `[u8; N]` with guaranteed 1-byte alignment.
///
/// This type implements the zerocopy traits (`FromBytes`, `IntoBytes`, `Immutable`, `KnownLayout`)
/// making it safe to transmute to/from byte arrays. It is primarily used internally by the
/// `ArrayFactory` type system to provide aligned arrays for POD unions.
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Clone, Copy)]
#[repr(transparent)]
pub struct U8Array<const N: usize>([u8; N]);

const _: () = assert!(align_of::<U8Array<0>>() == 1);

/// A transparent wrapper around `[u16; N]` with guaranteed 2-byte alignment.
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Clone, Copy)]
#[repr(transparent)]
pub struct U16Array<const N: usize>([u16; N]);

const _: () = assert!(align_of::<U16Array<0>>() == 2);

/// A transparent wrapper around `[u32; N]` with guaranteed 4-byte alignment.
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Clone, Copy)]
#[repr(transparent)]
pub struct U32Array<const N: usize>([u32; N]);

const _: () = assert!(align_of::<U32Array<0>>() == 4);

/// A transparent wrapper around `[u64; N]` with guaranteed 8-byte alignment.
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Clone, Copy)]
#[repr(transparent)]
pub struct U64Array<const N: usize>([u64; N]);

const _: () = assert!(align_of::<U64Array<0>>() == 8);

/// A type-level factory for creating aligned arrays based on alignment requirements.
///
/// This zero-sized type uses const generics to select the appropriate underlying array type
/// (`U8Array`, `U16Array`, `U32Array`, or `U64Array`) based on the alignment requirement `A` and
/// the number of elements `N`.
///
/// # Type Parameters
///
/// * `A` - The required alignment in bytes (1, 2, 4, or 8).
/// * `N` - The number of elements in the array.
///
/// # Examples
///
/// ```rust
/// use ostd_pod::array_helper::{ArrayFactory, ArrayManufacture};
///
/// // Creates a `U32Array<8>` (8 `u32` elements with 4-byte alignment)
/// type MyArray = <ArrayFactory<4, 8> as ArrayManufacture>::Array;
/// ```
pub enum ArrayFactory<const A: usize, const N: usize> {}

/// Trait that associates an `ArrayFactory` with its corresponding aligned array type.
///
/// This trait is implemented for `ArrayFactory<A, N>` where `A` is 1, 2, 4, or 8,
/// mapping to `U8Array`, `U16Array`, `U32Array`, and `U64Array` respectively.
pub trait ArrayManufacture {
    /// The aligned array type produced by this factory.
    type Array: FromBytes + IntoBytes + Immutable;
}

impl<const N: usize> ArrayManufacture for ArrayFactory<1, N> {
    type Array = U8Array<N>;
}

impl<const N: usize> ArrayManufacture for ArrayFactory<2, N> {
    type Array = U16Array<N>;
}

impl<const N: usize> ArrayManufacture for ArrayFactory<4, N> {
    type Array = U32Array<N>;
}

impl<const N: usize> ArrayManufacture for ArrayFactory<8, N> {
    type Array = U64Array<N>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8array_alignment() {
        assert_eq!(align_of::<U8Array<0>>(), 1);
        assert_eq!(align_of::<U8Array<1>>(), 1);
        assert_eq!(align_of::<U8Array<10>>(), 1);
    }

    #[test]
    fn u8array_size() {
        assert_eq!(size_of::<U8Array<0>>(), 0);
        assert_eq!(size_of::<U8Array<1>>(), 1);
        assert_eq!(size_of::<U8Array<4>>(), 4);
        assert_eq!(size_of::<U8Array<10>>(), 10);
    }

    #[test]
    fn u16array_alignment() {
        assert_eq!(align_of::<U16Array<0>>(), 2);
        assert_eq!(align_of::<U16Array<1>>(), 2);
        assert_eq!(align_of::<U16Array<10>>(), 2);
    }

    #[test]
    fn u16array_size() {
        assert_eq!(size_of::<U16Array<0>>(), 0);
        assert_eq!(size_of::<U16Array<1>>(), 2);
        assert_eq!(size_of::<U16Array<4>>(), 8);
        assert_eq!(size_of::<U16Array<10>>(), 20);
    }

    #[test]
    fn u32array_alignment() {
        assert_eq!(align_of::<U32Array<0>>(), 4);
        assert_eq!(align_of::<U32Array<1>>(), 4);
        assert_eq!(align_of::<U32Array<10>>(), 4);
    }

    #[test]
    fn u32array_size() {
        assert_eq!(size_of::<U32Array<0>>(), 0);
        assert_eq!(size_of::<U32Array<1>>(), 4);
        assert_eq!(size_of::<U32Array<4>>(), 16);
        assert_eq!(size_of::<U32Array<10>>(), 40);
    }

    #[test]
    fn u64array_alignment() {
        assert_eq!(align_of::<U64Array<0>>(), 8);
        assert_eq!(align_of::<U64Array<1>>(), 8);
        assert_eq!(align_of::<U64Array<10>>(), 8);
    }

    #[test]
    fn u64array_size() {
        assert_eq!(size_of::<U64Array<0>>(), 0);
        assert_eq!(size_of::<U64Array<1>>(), 8);
        assert_eq!(size_of::<U64Array<4>>(), 32);
        assert_eq!(size_of::<U64Array<10>>(), 80);
    }

    #[test]
    fn array_factory_1byte_alignment() {
        type Array = <ArrayFactory<1, 5> as ArrayManufacture>::Array;
        assert_eq!(align_of::<Array>(), 1);
        assert_eq!(size_of::<Array>(), 5);
    }

    #[test]
    fn array_factory_2byte_alignment() {
        type Array = <ArrayFactory<2, 5> as ArrayManufacture>::Array;
        assert_eq!(align_of::<Array>(), 2);
        assert_eq!(size_of::<Array>(), 10);
    }

    #[test]
    fn array_factory_4byte_alignment() {
        type Array = <ArrayFactory<4, 5> as ArrayManufacture>::Array;
        assert_eq!(align_of::<Array>(), 4);
        assert_eq!(size_of::<Array>(), 20);
    }

    #[test]
    fn array_factory_8byte_alignment() {
        type Array = <ArrayFactory<8, 5> as ArrayManufacture>::Array;
        assert_eq!(align_of::<Array>(), 8);
        assert_eq!(size_of::<Array>(), 40);
    }

    #[test]
    fn zerocopy_traits() {
        // Test that the types implement the required zerocopy traits
        fn assert_traits<T: FromBytes + IntoBytes + Immutable + KnownLayout>() {}

        assert_traits::<U16Array<4>>();
        assert_traits::<U32Array<4>>();
        assert_traits::<U64Array<4>>();
    }
}
