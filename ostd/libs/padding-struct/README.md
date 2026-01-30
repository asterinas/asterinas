# padding-struct

A Rust procedural macro for automatically adding explicit padding fields to `#[repr(C)]` structs.

## Overview

When working with `#[repr(C)]` structs, the Rust compiler automatically adds padding bytes to ensure proper alignment. The `#[padding_struct]` macro makes these padding bytes explicit by automatically generating padding fields in your struct definitions.

### Basic Example

```rust
use padding_struct::padding_struct;

#[repr(C)]
#[padding_struct]
struct MyStruct {
    a: u8,      // 1 byte
    b: u32,     // 4 bytes (aligned to 4-byte boundary)
    c: u16,     // 2 bytes
}
```

The macro generates a new struct with explicit paddings:

```rust
// Padded struct (the one you'll use)
#[repr(C)]
struct MyStruct {
    a: u8,
    __pad1: [u8; 3],  // padding before b
    b: u32,
    __pad2: [u8; 0],  // no padding before c
    c: u16,
    __pad3: [u8; 2],  // trailing padding
}
```

### Integration with zerocopy

The generated structs work seamlessly with `zerocopy` for safe transmutation:

```rust
use padding_struct::padding_struct;
use zerocopy::*;

#[repr(C)]
#[padding_struct]
#[derive(Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
struct SafeStruct {
    field1: u32,
    field2: u64,
}

// Now you can safely cast to/from bytes
let s = SafeStruct::new_zeroed();
let bytes: &[u8] = s.as_bytes();
```
