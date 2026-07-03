# Naming

### Be descriptive (`descriptive-names`) {#descriptive-names}

Choose names that convey meaning at the point of use.
Avoid single-letter names and ambiguous abbreviations.
Prefer full words over cryptic shorthand
so that readers do not need surrounding context
to understand a variable's purpose.
Prefer names that are as short as possible
while still being unambiguous at the point of use.

### Be accurate (`accurate-names`) {#accurate-names}

Avoid confusing names.
If a name can be misread
to imply the wrong meaning, behavior, or side effects,
it must be corrected immediately.

```rust
// Good — clearly a count
nr_deleted_watches: usize,
// Bad — looks like a collection
// rather than a numeric counter
deleted_watches: usize
```

Choose verbs that reflect the actual work being done.

```rust
impl PciCommonDevice {
    // Good — implies an MMIO read is involved
    pub fn read_command(&self) -> Command { /* .. */ }
    // Bad — looks like a plain field access
    pub fn command(&self) -> Command { /* .. */ }
}
```

```rust
mod char_device {
    // Good — implies an O(n) collection pass
    pub fn collect_all() -> Vec<Arc<dyn Device>> { /* .. */ }
    // Bad — sounds like an accessor returning a reference
    pub fn get_all() -> Vec<Arc<dyn Device>> { /* .. */ }
}
```

See also:
PR [#1488](https://github.com/asterinas/asterinas/pull/1488#discussion_r1825441287)
and [#2964](https://github.com/asterinas/asterinas/pull/2964#discussion_r2789739882).

### Encode units and important attributes in names (`encode-units`) {#encode-units}

When the type does not encode the unit,
the name must.
Kernel code deals with bytes, pages, frames,
nanoseconds, ticks, and sectors —
ambiguous units are a source of real bugs.

```text
// Good — unit is unambiguous
timeout_ns
offset_bytes
size_pages
delay_ms

// Bad — unit is ambiguous
timeout
offset
size
delay
```

Where the language's type system can enforce units (e.g., newtypes),
prefer that.
Where it cannot, the name must carry the information.

See also:
PR [#2796](https://github.com/asterinas/asterinas/pull/2796#discussion_r2646889913).

### No magic number (`no-magic-number`) {#no-magic-number}

Numeric literals must be meaningful at the point of use.
When a number represents a non-local invariant,
an external contract,
or a domain-specific meaning
beyond its immediate arithmetic value,
give that meaning a name.
Use a constant,
typed value,
enum variant,
or helper function,
whichever best expresses the invariant.

```rust
// Good — the flag's meaning is explicit.
const NEEDS_ACK_FLAG: u8 = 0b0000_0100;
let needs_ack = (packet_flags & NEEDS_ACK_FLAG) != 0;

// Bad — the reader must infer why this bit is special.
let needs_ack = (packet_flags & 0b0000_0100) != 0;
```

Prefer deriving related values from the named source
instead of repeating the same number in multiple places.
If the name alone does not explain where the value comes from,
add a short comment or cite the relevant specification.

Do not introduce names for numbers
whose meaning is already obvious locally,
such as `0`, `1`, or `2`
in ordinary arithmetic,
indexing,
small ranges,
or direct comparisons.

### Use assertion-style boolean names (`bool-names`) {#bool-names}

Boolean variables and functions
should read as assertions of fact.
Use `is_`, `has_`, `can_`, `should_`, `was_`,
or `needs_` prefixes.
Never use negated names
(`is_not_empty`, `no_error`);
prefer the positive form
(`is_empty`, `ok` or `succeeded`).
A bare name like `found`, `done`, or `ready`
is acceptable when the context is unambiguous.

```rust
// Good — reads as an assertion
fn is_page_aligned(&self) -> bool { ... }
fn has_permission(&self, perm: Permission) -> bool { ... }
let can_read = mode.is_readable();

// Bad — verb suggests an action, not a query
fn check_permission(&self, perm: Permission) -> bool { ... }
// Bad — negated name
let is_not_empty = !buf.is_empty();
```

See also:
PR [#1488](https://github.com/asterinas/asterinas/pull/1488#discussion_r1841827039).

### Format error messages consistently (`error-message-format`) {#error-message-format}

Start with a lowercase letter
(unless the first word is a proper noun or identifier).
Be specific:
prefer "`len` is too large" over "the argument is invalid".

For system call errors,
follow the style and descriptions in Linux man pages.
