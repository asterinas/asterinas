// SPDX-License-Identifier: MPL-2.0

use core::mem::offset_of;

use padding_struct::padding_struct;

/// Test basic padding functionality
#[test]
fn basic_padding() {
    #[repr(C)]
    #[padding_struct]
    struct TestStruct {
        a: u8,
        b: u32,
        c: u16,
    }

    // Verify reference struct exists
    let _ref_struct = __TestStruct__ { a: 1, b: 2, c: 3 };

    // Verify padded struct
    let padded = TestStruct {
        a: 1,
        __pad1: [0; {
            offset_of!(__TestStruct__, b) - offset_of!(__TestStruct__, a) - size_of::<u8>()
        }],
        b: 2,
        __pad2: [0; {
            offset_of!(__TestStruct__, c) - offset_of!(__TestStruct__, b) - size_of::<u32>()
        }],
        c: 3,
        __pad3: [0; {
            size_of::<__TestStruct__>() - offset_of!(__TestStruct__, c) - size_of::<u16>()
        }],
    };

    assert_eq!(padded.a, 1);
    assert_eq!(padded.b, 2);
    assert_eq!(padded.c, 3);
}

/// Test single field struct
#[test]
fn single_field() {
    #[repr(C)]
    #[padding_struct]
    struct SingleField {
        value: u64,
    }

    let single = SingleField {
        value: 42,
        __pad1: [0; {
            size_of::<__SingleField__>() - offset_of!(__SingleField__, value) - size_of::<u64>()
        }],
    };

    assert_eq!(single.value, 42);
}

/// Test multiple fields struct
#[test]
fn multiple_fields() {
    #[repr(C)]
    #[padding_struct]
    struct MultiField {
        a: u8,
        b: u16,
        c: u32,
        d: u64,
    }

    let multi = MultiField {
        a: 1,
        __pad1: [0; {
            offset_of!(__MultiField__, b) - offset_of!(__MultiField__, a) - size_of::<u8>()
        }],
        b: 2,
        __pad2: [0; {
            offset_of!(__MultiField__, c) - offset_of!(__MultiField__, b) - size_of::<u16>()
        }],
        c: 3,
        __pad3: [0; {
            offset_of!(__MultiField__, d) - offset_of!(__MultiField__, c) - size_of::<u32>()
        }],
        d: 4,
        __pad4: [0; {
            size_of::<__MultiField__>() - offset_of!(__MultiField__, d) - size_of::<u64>()
        }],
    };

    assert_eq!(multi.a, 1);
    assert_eq!(multi.b, 2);
    assert_eq!(multi.c, 3);
    assert_eq!(multi.d, 4);
}

/// Test struct with field documentation
#[test]
fn with_field_docs() {
    #[repr(C)]
    #[padding_struct]
    struct Documented {
        /// First field
        first: u8,
        /// Second field
        second: u32,
    }

    let doc = Documented {
        first: 10,
        __pad1: [0; {
            offset_of!(__Documented__, second) - offset_of!(__Documented__, first) - size_of::<u8>()
        }],
        second: 20,
        __pad2: [0; {
            size_of::<__Documented__>() - offset_of!(__Documented__, second) - size_of::<u32>()
        }],
    };

    assert_eq!(doc.first, 10);
    assert_eq!(doc.second, 20);
}

/// Verify that field offsets are consistent between reference and padded structs
#[test]
fn offset_consistency() {
    #[repr(C)]
    #[padding_struct]
    struct OffsetTest {
        a: u8,
        b: u32,
    }

    // Reference struct offsets
    let ref_a_offset = offset_of!(__OffsetTest__, a);
    let ref_b_offset = offset_of!(__OffsetTest__, b);

    // Padded struct offsets should be the same
    let padded_a_offset = offset_of!(OffsetTest, a);
    let padded_b_offset = offset_of!(OffsetTest, b);

    assert_eq!(ref_a_offset, padded_a_offset);
    assert_eq!(ref_b_offset, padded_b_offset);
}

/// Test that padding is zero-filled
#[test]
fn padding_zero_filled() {
    #[repr(C)]
    #[padding_struct]
    struct ZeroPadded {
        a: u8,
        b: u32,
    }

    let zero_pad = ZeroPadded {
        a: 255,
        __pad1: [0; {
            offset_of!(__ZeroPadded__, b) - offset_of!(__ZeroPadded__, a) - size_of::<u8>()
        }],
        b: 0xFFFFFFFF,
        __pad2: [0; {
            size_of::<__ZeroPadded__>() - offset_of!(__ZeroPadded__, b) - size_of::<u32>()
        }],
    };

    // Verify padding is all zeros
    for byte in &zero_pad.__pad1 {
        assert_eq!(*byte, 0);
    }
    for byte in &zero_pad.__pad2 {
        assert_eq!(*byte, 0);
    }
}

/// Test that padded struct can derive zerocopy traits
#[test]
fn zerocopy_derive() {
    use zerocopy::*;

    #[repr(C)]
    #[padding_struct]
    #[derive(Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
    struct ZerocopyStruct {
        a: u8,
        b: u32,
        c: u16,
    }

    // Test Zeroable
    let zeroed = ZerocopyStruct::new_zeroed();
    assert_eq!(zeroed.a, 0);
    assert_eq!(zeroed.b, 0);
    assert_eq!(zeroed.c, 0);

    // Test Pod - cast from bytes
    let bytes = [1u8, 0, 0, 0, 0x12, 0x34, 0x56, 0x78, 0xAB, 0xCD, 0, 0];
    let from_bytes: &ZerocopyStruct =
        FromBytes::ref_from_bytes(&bytes[..size_of::<ZerocopyStruct>()]).unwrap();
    assert_eq!(from_bytes.a, 1);
    assert_eq!(from_bytes.b, 0x78563412);
    assert_eq!(from_bytes.c, 0xCDAB);

    // Test Pod - cast to bytes
    let test_struct = ZerocopyStruct {
        a: 42,
        __pad1: [0; {
            offset_of!(__ZerocopyStruct__, b) - offset_of!(__ZerocopyStruct__, a) - size_of::<u8>()
        }],
        b: 0xDEADBEEF,
        __pad2: [0; {
            offset_of!(__ZerocopyStruct__, c) - offset_of!(__ZerocopyStruct__, b) - size_of::<u32>()
        }],
        c: 0x1234,
        __pad3: [0; {
            size_of::<__ZerocopyStruct__>() - offset_of!(__ZerocopyStruct__, c) - size_of::<u16>()
        }],
    };
    let as_bytes: &[u8] = test_struct.as_bytes();
    assert_eq!(as_bytes[0], 42);
}

/// Test that repr attributes (align, packed, etc.) are preserved in ref struct
#[test]
fn repr_align_preserved() {
    #[repr(C, align(16))]
    #[padding_struct]
    struct AlignedStruct {
        a: u8,
        b: u32,
    }

    // Verify alignment is correct
    assert_eq!(align_of::<AlignedStruct>(), 16);
    assert_eq!(align_of::<__AlignedStruct__>(), 16);

    let aligned = AlignedStruct {
        a: 1,
        __pad1: [0; {
            offset_of!(__AlignedStruct__, b) - offset_of!(__AlignedStruct__, a) - size_of::<u8>()
        }],
        b: 2,
        __pad2: [0; {
            size_of::<__AlignedStruct__>() - offset_of!(__AlignedStruct__, b) - size_of::<u32>()
        }],
    };

    assert_eq!(aligned.a, 1);
    assert_eq!(aligned.b, 2);
}

/// Test that size and alignment match between ref struct and padded struct
#[test]
fn size_align_match() {
    #[repr(C, align(8))]
    #[padding_struct]
    struct TestStruct {
        a: u8,
        b: u16,
        c: u32,
    }

    // The compile-time check ensures these are equal
    assert_eq!(size_of::<TestStruct>(), size_of::<__TestStruct__>());
    assert_eq!(align_of::<TestStruct>(), align_of::<__TestStruct__>());
}
