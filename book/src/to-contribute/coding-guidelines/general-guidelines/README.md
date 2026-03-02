# General Guidelines

## Names

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
    // Good — implies a MMIO read is involved
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

## Comments

### Prefer semantic line breaks (`semantic-line-breaks`) {#semantic-line-breaks}

For prose in Markdown and doc comments,
insert line breaks at semantic boundaries
so each line carries one coherent idea.
At minimum, break at sentence boundaries.
For longer sentences, also consider breaking at clause boundaries.

Semantic line breaks make diffs smaller,
reviews easier,
and merge conflicts less noisy.

As an exception,
RFC documents that are mostly read-only
can use regular paragraph wrapping.

See also:
[Semantic Line Breaks](https://sembr.org/).

### Explain why, not what (`explain-why`) {#explain-why}

Comments should explain the intent behind the code,
not restate what the code does.
If a comment merely paraphrases the code,
it adds noise without insight.

If a comment is needed to explain what code does,
first try to rewrite the code.
Do not write good comments to compensate for bad code —
rewrite it to be straightforward.

See also:
_The Art of Readable Code_, Chapter 6 "Knowing What to Comment";
PR [#2265](https://github.com/asterinas/asterinas/pull/2265#discussion_r2266220943)
and [#2050](https://github.com/asterinas/asterinas/pull/2050#discussion_r2224106025).

### Document design decisions (`design-decisions`) {#design-decisions}

When the code makes a non-obvious choice —
a particular data structure, a locking strategy,
a deviation from Linux behavior —
add a comment explaining the rationale
and any alternatives considered.
Design-decision comments ("director's commentary")
are the most valuable kind of comment.

```rust
// We use a radix tree rather than a HashMap
// because lookups must be O(log n) worst-case
// for the page fault handler.
// A HashMap gives O(1) amortized
// but O(n) worst-case due to rehashing,
// which is unacceptable on the page fault path.
```

See also:
PR [#2265](https://github.com/asterinas/asterinas/pull/2265#discussion_r2266220943)
and [#2050](https://github.com/asterinas/asterinas/pull/2050#discussion_r2224106025).

### Cite specifications and algorithm sources (`cite-sources`) {#cite-sources}

When implementing behavior defined by
an external specification or a non-trivial algorithm,
cite the source:
the relevant POSIX section, Linux man page,
hardware reference manual, or academic paper.

```rust
/// Maximum number of bytes guaranteed to be written to a pipe atomically.
///
/// For more details, see the description of `PIPE_BUF` in
/// <https://man7.org/linux/man-pages/man7/pipe.7.html>.
const PIPE_BUF: usize = 4096;
```

## Layout

### One concept per file (`one-concept-per-file`) {#one-concept-per-file}

When a file grows long or contains multiple distinct concepts,
split it.
Each major data structure, each subsystem entry point,
each significant abstraction
deserves its own file.

### Organize code for top-down reading (`top-down-reading`) {#top-down-reading}

A source file should read from top to bottom.
Start with high-level entry points and core flow.
Move implementation details downward
so readers can understand the big picture first
before diving into low-level helpers.

Within each visibility group (e.g., a module),
order methods so that callers appear before callees where possible,
enabling the file to be read top to bottom.
Place public methods before private helpers.

### Group statements into logical paragraphs (`logical-paragraphs`) {#logical-paragraphs}

Within functions,
group related statements into logical paragraphs
separated by blank lines.
Each paragraph should represent one sub-step
of the function's overall purpose.

For long functions,
add a one-line summary comment
at the start of each paragraph
when the paragraph intent is not obvious.

## Formatting

### Format error messages consistently (`error-message-format`) {#error-message-format}

Start with a lowercase letter
(unless the first word is a proper noun or identifier).
Be specific:
prefer "`len` is too large" over "the argument is invalid".

For system call errors,
follow the style and descriptions in Linux man pages.

## API Design

### Stick to familiar conventions (`familiar-conventions`) {#familiar-conventions}

Prefer names and API shapes
that users already know from Rust and Linux.
Do not invent new terms
for well-known operations.

```rust
// Good — follows common Rust naming conventions
pub fn len(&self) -> usize { ... }
pub fn as_ptr(&self) -> *const u8 { ... }

// Bad — unfamiliar synonyms for common operations
pub fn length(&self) -> usize { ... }
pub fn to_pointer(&self) -> *const u8 { ... }
```

See also:
[Least Surprise](../how-guidelines-are-written.md#least-surprise).

### Hide implementation details (`hide-impl-details`) {#hide-impl-details}

Do not expose internal implementation details
through public APIs (including their documentation).
A module's public surface
should contain only what its consumers need.

See also:
[Modules and Crates](../rust-guidelines/language-items/modules-and-crates.md#narrow-visibility)
for Rust-specific visibility rules;
PR [#2951](https://github.com/asterinas/asterinas/pull/2951#discussion_r2786925410).

### Validate at boundaries, trust internally (`validate-at-boundaries`) {#validate-at-boundaries}

Designate certain interfaces as validation boundaries.
In Asterinas, syscall entry points
are the primary boundary:
all user-supplied data
(pointers, file descriptors, sizes, flags, strings)
must be validated at the syscall boundary.
Once validated, internal kernel functions
may trust these values without re-validation.

See also:
PR [#2806](https://github.com/asterinas/asterinas/pull/2806).
