# Types & Traits

### Use types to enforce invariants (`rust-type-invariants`) {#rust-type-invariants}

Leverage the type system
to make illegal states _unrepresentable_.

Define newtypes to encode domain constraints.

```rust
// Good — a `Nice` value is guaranteed to be valid
pub struct Nice(NiceValue);
type NiceValue = RangedI8<-20, 19>;

// Bad — `i8` admits invalid values for nice levels
pub type Nice = i8;
```

Prefer enums over bare integers and boolean flags.

```rust
// Good — access mode is constrained by the enum
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    O_RDONLY = 0,
    O_WRONLY = 1,
    O_RDWR = 2,
}

// Bad — `u8` admits invalid values
pub type AccessMode = u8;
```

Encode invariants in generic parameters where needed.

```rust
impl IoMem<Sensitive> {
    // Good — only unsafe code can write to sensitive MMIO
    pub unsafe fn write_u32(&self, offset: usize, new_val: u32) { /* .. */ }
}

impl IoMem<Insensitive> {
    // Good — safe code can write to insensitive MMIO
    pub fn write_u32(&self, offset: usize, new_val: u32) { /* .. */ }
}

pub enum Sensitive {}
pub enum Insensitive {}
```

Asterinas uses this pattern widely,
for example with newtypes such as `CpuId`
and `AlignedUsize<const N: u16>`.

See also:
PR [#2265](https://github.com/asterinas/asterinas/pull/2265#discussion_r2266214191)
and [#2514](https://github.com/asterinas/asterinas/pull/2514).

### Prefer enum over trait objects for closed sets (`enum-over-dyn`) {#enum-over-dyn}

When the set of variants is known and closed,
an enum is often preferable to `Box<dyn Trait>`
for both performance and pattern-matching expressiveness.

```rust
// Good — closed set modeled as an enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermStatus {
    Exited(u8),
    Killed(SigNum),
}
```

### Encapsulate fields behind getters (`getter-encapsulation`) {#getter-encapsulation}

Do not make fields public
when a simple getter method would do.
A getter preserves naming flexibility
and leaves room for future invariants.

```rust
// Good — field is private, accessed via getter
pub struct Vma {
    perms: VmPerms,
}

impl Vma {
    pub fn perms(&self) -> VmPerms {
        self.perms
    }
}

// Bad — public field exposes representation
pub struct Vma {
    pub perms: VmPerms,
}
```

### Define bit flags types outside the macro for custom derives (`bitflags-derives`) {#bitflags-derives}

When a bit flags type needs a custom derive that `bitflags` does not natively support,
define an integer wrapper outside `bitflags!`
and use the macro's `impl` syntax to generate the flags API.

```rust
use ostd_pod::IntoBytes;

/// Device status flags serialized in a reply.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, IntoBytes)]
pub struct DeviceStatus(u32);

bitflags::bitflags! {
    impl DeviceStatus: u32 {
        const READY = 1 << 0;
    }
}
```

See also:
[`bitflags` custom derives](https://docs.rs/bitflags/2.13.0/bitflags/#custom-derives)
and issue [#4120](https://github.com/asterinas/asterinas/issues/4120).

### Keep bit flags declarations consistent (`bitflags-layout`) {#bitflags-layout}

Use spaces to align the `=` signs and values within each flags type,
including unnamed masks.

Within each flags type, use either bit shifts or numeric literals consistently.
Zero values and unnamed masks are exempt,
as are composite expressions and references to other flags.

```rust
bitflags::bitflags! {
    /// Access permissions for a mapping.
    #[derive(Clone, Copy, Debug)]
    pub struct MappingPerms: u32 {
        const NONE      = 0;
        const READ      = 1 << 0;
        const WRITE     = 1 << 1;
        const EXEC      = 1 << 2;
        const ALL_PERMS = Self::READ.bits() | Self::WRITE.bits() | Self::EXEC.bits();
    }
}
```

### Declare the bits that `bitflags` must retain (`bitflags-retained-bits`) {#bitflags-retained-bits}

In a `bitflags!` declaration, use an unnamed mask for uninterpreted bits that must be retained:
use `const _ = !0;` for all bits, or a narrower mask for a subset.
Explain which bit patterns the type accepts next to the mask.

```rust
bitflags::bitflags! {
    /// A register snapshot used for read-modify-write updates.
    #[derive(Clone, Copy, Debug)]
    pub struct RegisterFlags: u32 {
        const ENABLED = 1 << 0;

        // This type accepts all bit patterns,
        // although only some flags are recognized.
        const _       = !0;
    }
}
```

With `const _ = !0`, `from_bits` accepts every bit pattern,
so callers must check supported options separately when required.

See also:
[`bitflags` externally defined flags](https://docs.rs/bitflags/2.13.0/bitflags/#externally-defined-flags).
