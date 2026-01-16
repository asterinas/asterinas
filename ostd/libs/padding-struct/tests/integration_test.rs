// SPDX-License-Identifier: MPL-2.0

use padding_struct::padding_struct;

/// Test basic padding functionality
#[test]
fn test_basic_padding() {
    #[repr(C)]
    #[padding_struct]
    struct TestStruct {
        a: u8,
        b: u32,
        c: u16,
    }

    // Verify reference struct exists
    let _ref_struct = __TestStruct { a: 1, b: 2, c: 3 };

    // Verify padded struct
    let padded = TestStruct {
        a: 1,
        __pad1: [0; {
            core::mem::offset_of!(__TestStruct, b)
                - core::mem::offset_of!(__TestStruct, a)
                - core::mem::size_of::<u8>()
        }],
        b: 2,
        __pad2: [0; {
            core::mem::offset_of!(__TestStruct, c)
                - core::mem::offset_of!(__TestStruct, b)
                - core::mem::size_of::<u32>()
        }],
        c: 3,
        __pad3: [0; {
            core::mem::size_of::<__TestStruct>()
                - core::mem::offset_of!(__TestStruct, c)
                - core::mem::size_of::<u16>()
        }],
    };

    assert_eq!(padded.a, 1);
    assert_eq!(padded.b, 2);
    assert_eq!(padded.c, 3);
}

/// Test size parameter with literal
#[test]
fn test_with_size_literal() {
    #[repr(C)]
    #[padding_struct(size = 64)]
    struct FixedSize {
        value: u32,
    }

    // Verify struct size is 64 bytes
    assert_eq!(core::mem::size_of::<FixedSize>(), 64);
}

/// Test size parameter with constant expression
#[test]
fn test_with_size_expression() {
    const PAGE_SIZE: usize = 4096;

    #[repr(C)]
    #[padding_struct(size = PAGE_SIZE)]
    struct PageAligned {
        header: u64,
        data: [u8; 32],
    }

    // Verify struct size is 4096 bytes
    assert_eq!(core::mem::size_of::<PageAligned>(), PAGE_SIZE);
}

/// Test size parameter with size_of expression
#[test]
fn test_with_sizeof_expression() {
    #[repr(C)]
    #[padding_struct(size = core::mem::size_of::<[u8; 128]>())]
    struct CacheLine {
        counter: u64,
        flag: u32,
    }

    // Verify struct size is 128 bytes
    assert_eq!(core::mem::size_of::<CacheLine>(), 128);
}

/// Test single field struct
#[test]
fn test_single_field() {
    #[repr(C)]
    #[padding_struct]
    struct SingleField {
        value: u64,
    }

    let single = SingleField {
        value: 42,
        __pad1: [0; {
            core::mem::size_of::<__SingleField>()
                - core::mem::offset_of!(__SingleField, value)
                - core::mem::size_of::<u64>()
        }],
    };

    assert_eq!(single.value, 42);
}

/// Test multiple fields struct
#[test]
fn test_multiple_fields() {
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
            core::mem::offset_of!(__MultiField, b)
                - core::mem::offset_of!(__MultiField, a)
                - core::mem::size_of::<u8>()
        }],
        b: 2,
        __pad2: [0; {
            core::mem::offset_of!(__MultiField, c)
                - core::mem::offset_of!(__MultiField, b)
                - core::mem::size_of::<u16>()
        }],
        c: 3,
        __pad3: [0; {
            core::mem::offset_of!(__MultiField, d)
                - core::mem::offset_of!(__MultiField, c)
                - core::mem::size_of::<u32>()
        }],
        d: 4,
        __pad4: [0; {
            core::mem::size_of::<__MultiField>()
                - core::mem::offset_of!(__MultiField, d)
                - core::mem::size_of::<u64>()
        }],
    };

    assert_eq!(multi.a, 1);
    assert_eq!(multi.b, 2);
    assert_eq!(multi.c, 3);
    assert_eq!(multi.d, 4);
}

/// Test struct with field documentation
#[test]
fn test_with_field_docs() {
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
            core::mem::offset_of!(__Documented, second)
                - core::mem::offset_of!(__Documented, first)
                - core::mem::size_of::<u8>()
        }],
        second: 20,
        __pad2: [0; {
            core::mem::size_of::<__Documented>()
                - core::mem::offset_of!(__Documented, second)
                - core::mem::size_of::<u32>()
        }],
    };

    assert_eq!(doc.first, 10);
    assert_eq!(doc.second, 20);
}

/// Verify that field offsets are consistent between reference and padded structs
#[test]
fn test_offset_consistency() {
    #[repr(C)]
    #[padding_struct]
    struct OffsetTest {
        a: u8,
        b: u32,
    }

    // Reference struct offsets
    let ref_a_offset = core::mem::offset_of!(__OffsetTest, a);
    let ref_b_offset = core::mem::offset_of!(__OffsetTest, b);

    // Padded struct offsets should be the same
    let padded_a_offset = core::mem::offset_of!(OffsetTest, a);
    let padded_b_offset = core::mem::offset_of!(OffsetTest, b);

    assert_eq!(ref_a_offset, padded_a_offset);
    assert_eq!(ref_b_offset, padded_b_offset);
}

/// Test that padding is zero-filled
#[test]
fn test_padding_zero_filled() {
    #[repr(C)]
    #[padding_struct]
    struct ZeroPadded {
        a: u8,
        b: u32,
    }

    let zero_pad = ZeroPadded {
        a: 255,
        __pad1: [0; {
            core::mem::offset_of!(__ZeroPadded, b)
                - core::mem::offset_of!(__ZeroPadded, a)
                - core::mem::size_of::<u8>()
        }],
        b: 0xFFFFFFFF,
        __pad2: [0; {
            core::mem::size_of::<__ZeroPadded>()
                - core::mem::offset_of!(__ZeroPadded, b)
                - core::mem::size_of::<u32>()
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

/// Test with large size parameter
#[test]
fn test_large_size() {
    #[repr(C)]
    #[padding_struct(size = 8192)]
    struct LargeStruct {
        id: u32,
        value: u64,
    }

    assert_eq!(core::mem::size_of::<LargeStruct>(), 8192);
}

/// Test that padded struct can derive bytemuck traits (Pod and Zeroable)
#[test]
fn test_bytemuck_derive() {
    use bytemuck::{Pod, Zeroable};

    #[repr(C)]
    #[padding_struct]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct BytemuckStruct {
        a: u8,
        b: u32,
        c: u16,
    }

    // Test Zeroable
    let zeroed = BytemuckStruct::zeroed();
    assert_eq!(zeroed.a, 0);
    assert_eq!(zeroed.b, 0);
    assert_eq!(zeroed.c, 0);

    // Test Pod - cast from bytes
    let bytes = [1u8, 0, 0, 0, 0x12, 0x34, 0x56, 0x78, 0xAB, 0xCD, 0, 0];
    let from_bytes: &BytemuckStruct =
        bytemuck::from_bytes(&bytes[..core::mem::size_of::<BytemuckStruct>()]);
    assert_eq!(from_bytes.a, 1);
    assert_eq!(from_bytes.b, 0x78563412);
    assert_eq!(from_bytes.c, 0xCDAB);

    // Test Pod - cast to bytes
    let test_struct = BytemuckStruct {
        a: 42,
        __pad1: [0; {
            core::mem::offset_of!(__BytemuckStruct, b)
                - core::mem::offset_of!(__BytemuckStruct, a)
                - core::mem::size_of::<u8>()
        }],
        b: 0xDEADBEEF,
        __pad2: [0; {
            core::mem::offset_of!(__BytemuckStruct, c)
                - core::mem::offset_of!(__BytemuckStruct, b)
                - core::mem::size_of::<u32>()
        }],
        c: 0x1234,
        __pad3: [0; {
            core::mem::size_of::<__BytemuckStruct>()
                - core::mem::offset_of!(__BytemuckStruct, c)
                - core::mem::size_of::<u16>()
        }],
    };
    let as_bytes: &[u8] = bytemuck::bytes_of(&test_struct);
    assert_eq!(as_bytes[0], 42);
}

/// Test bytemuck with fixed size struct
#[test]
fn test_bytemuck_fixed_size() {
    use bytemuck::{Pod, Zeroable};

    #[repr(C)]
    #[padding_struct(size = 256)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct FixedSizePod {
        header: u64,
        data: u32,
    }

    assert_eq!(core::mem::size_of::<FixedSizePod>(), 256);

    // Test that we can cast from a zeroed byte array
    let bytes = [0u8; 256];
    let from_bytes: &FixedSizePod = bytemuck::from_bytes(&bytes);
    assert_eq!(from_bytes.header, 0);
    assert_eq!(from_bytes.data, 0);

    // Test Zeroable
    let zeroed: FixedSizePod = Zeroable::zeroed();
    assert_eq!(core::mem::size_of_val(&zeroed), 256);
}
