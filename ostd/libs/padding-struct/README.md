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

The macro generates:

```rust
// Reference struct (used for offset calculations)
#[repr(C)]
struct __MyStruct {
    a: u8,
    b: u32,
    c: u16,
}

// Padded struct (the one you'll use)
#[repr(C)]
struct MyStruct {
    a: u8,
    _pad1: [u8; 3],  // padding before b
    b: u32,
    _pad2: [u8; 0],  // no padding before c
    c: u16,
    _pad3: [u8; 2],  // trailing padding
}
```

### Fixed Size Structs

Specify a total size to pad the struct to a specific size:

```rust
use padding_struct::padding_struct;

#[repr(C)]
#[padding_struct(size = 4096)]
struct PageAligned {
    header: u64,
    data: [u8; 32],
}
// Total size will be exactly 4096 bytes
```

### Integration with bytemuck

The generated structs work seamlessly with `bytemuck` for safe transmutation:

```rust
use padding_struct::padding_struct;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
#[padding_struct]
struct SafeStruct {
    field1: u32,
    field2: u64,
}

// Now you can safely cast to/from bytes
let s = SafeStruct::zeroed();
let bytes: &[u8] = bytemuck::bytes_of(&s);
```
