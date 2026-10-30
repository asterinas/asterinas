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

### Derive the traits a flags type needs (`bitflags-derives`) {#bitflags-derives}

Derive `Clone`, `Copy`, and `Debug` by default for `bitflags` types.
Flags are small values that callers copy and inspect in diagnostics.
Add other traits, such as `Eq`, `Hash`, `Ord`, `PartialEq`, and `PartialOrd`,
only when their uses require them.
Follow the usual [attribute and derive ordering](attributes-and-macros.md#alphabetical-attrs).

Derive `Pod` only for raw flags that accept every bit pattern.
Define an integer wrapper outside `bitflags!`
and add the flags API with `impl` syntax.
Document why direct byte conversion is appropriate
and where supported options are validated.
For inputs requiring validation, use integer ABI fields and explicit conversion instead.

See also:
[`bitflags` custom derives](https://docs.rs/bitflags/2.13.0/bitflags/#custom-derives)
and issue [#4120](https://github.com/asterinas/asterinas/issues/4120).

### Keep flags declarations consistent (`bitflags-layout`) {#bitflags-layout}

Within each flags type,
vertically align the `=` signs and the starts of their values using spaces.
Include unnamed masks in the alignment,
and align existing end-of-line comments with spaces as well.
Do not depend on tab width.

Keep numeric notation consistent within a type;
different types need not use the same notation.
For example, octal is appropriate for Unix permissions.
Preserve an existing consistent notation.
When using shifts for single-bit flags, write `1 << n` rather than `2 << (n - 1)`.
Multi-bit fields may use shifts such as `3 << 8` or `0xff << 8`.
Zero may appear alongside either shifts or literal values.
Composite expressions and references to other flags are exempt from the numeric notation rule.

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

### Declare the bits that must be retained (`bitflags-retained-bits`) {#bitflags-retained-bits}

When a register snapshot or protocol record must preserve uninterpreted bits,
include those bits in the flags declaration.
Use an unnamed mask, `const _ = !0;`, if every bit must be retained,
or a narrower mask if only a subset must survive.
Explain the requirement next to the mask.

```rust
bitflags::bitflags! {
    /// A register snapshot used for read-modify-write updates.
    #[derive(Clone, Copy, Debug)]
    pub struct RegisterFlags: u32 {
        const ENABLED = 1 << 0;

        // Register updates must preserve bits not interpreted here.
        const _       = !0;
    }
}
```

`from_bits_retain` alone does not establish this policy for the type:
a later `from_bits_truncate` can still discard undeclared bits.
An unnamed mask makes those bits known to `bitflags`,
so constructors and truncating operations such as `!` account for them.
With `const _ = !0`, `from_bits` accepts every input bit pattern;
the mask does not mean every option is supported.
Validate supported options separately.

See also:
[`bitflags` externally defined flags](https://docs.rs/bitflags/2.13.0/bitflags/#externally-defined-flags);
issue [#4120](https://github.com/asterinas/asterinas/issues/4120).
