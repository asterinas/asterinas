# Design

### Give each unit a single responsibility (`single-responsibility`) {#single-responsibility}

Each module, type, or function
should have one, and only one, reason to change.
If you cannot describe what a unit does
without the words "and," "or," or "but,"
it has too many responsibilities.

This applies at every scale:

- **Functions** should do one thing, do it well, and do it only.
  If you can extract another function from it
  with a name that is not merely a restatement of its implementation,
  the original function is doing more than one thing.
  Do not mix levels of abstraction:
  a syscall handler should read like a specification,
  while byte-level manipulation belongs in a helper.
- **Files** should hold one concept.
  When a file grows long or contains multiple distinct concepts,
  split it — each major data structure, subsystem entry point,
  or significant abstraction deserves its own file.

```rust
// Good — each function operates at one level of abstraction
pub fn sys_connect(sockfd: i32, addr: Vaddr, len: u32) -> Result<()> {
    let socket = get_socket(sockfd)?;
    let remote_addr = parse_socket_addr(addr, len)?;
    socket.connect(remote_addr)
}

// Bad — mixes high-level logic with low-level details
pub fn sys_connect(sockfd: i32, addr: Vaddr, len: u32) -> Result<()> {
    let fd_table = current_process().fd_table().lock();
    let file = fd_table.get(sockfd).ok_or(Errno::EBADF)?;
    let socket = file.downcast_ref::<Socket>().ok_or(Errno::ENOTSOCK)?;
    let bytes = read_bytes_from_user(addr, len as usize)?;
    let family = u16::from_ne_bytes([bytes[0], bytes[1]]);
    // ... 30 more lines of byte parsing ...
}
```

See also:
_Clean Code_, Chapter 3 "Functions";
PR [#639](https://github.com/asterinas/asterinas/pull/639#discussion_r1524629393).

### Don't repeat yourself (`dry`) {#dry}

Every piece of knowledge
should have a single, unambiguous representation.
Duplication harms readability and maintainability.
When the same pattern appears three or more times,
eliminate the duplication (e.g., by adding a helper function).

### Hide details behind interfaces (`information-hiding`) {#information-hiding}

Hide details behind well-defined interfaces.
A module's public surface should contain
only what its consumers need.
Internal data structures, helper types,
and bookkeeping fields should remain private,
and implementation details should not leak
through public APIs (including their documentation).

See also:
[`narrow-visibility`](rust-specific/crates-and-modules.md#narrow-visibility)
for Rust-specific visibility rules;
PR [#2951](https://github.com/asterinas/asterinas/pull/2951#discussion_r2786925410).

### Be open for extension, closed for modification (`open-closed`) {#open-closed}

Stable modules and APIs should be
open to extension
but closed to breaking modification.
Prefer adding new behavior
through existing interfaces
(traits, enums, and pluggable components)
instead of repeatedly editing established call paths.
Do not introduce extension points preemptively;
add them when there is a concrete extension need.

### Follow the principle of least surprise (`least-surprise`) {#least-surprise}

Functions, types, and APIs should behave
as their names and signatures suggest.
When an obvious behavior is not implemented,
readers lose trust in the codebase
and must fall back on reading implementation details.
Prefer names and API shapes
that users already know from Rust and Linux;
do not invent new terms for well-known operations.

```rust
// Good — follows common Rust naming conventions
pub fn len(&self) -> usize { ... }
pub fn as_ptr(&self) -> *const u8 { ... }

// Bad — unfamiliar synonyms for common operations
pub fn length(&self) -> usize { ... }
pub fn to_pointer(&self) -> *const u8 { ... }
```

### Aim for loose coupling and strong cohesion (`coupling-cohesion`) {#coupling-cohesion}

Connections between modules should be
small, visible, and flexible.
Within a module, every part should contribute
to a single, well-defined purpose.

### Be consistent (`consistency`) {#consistency}

Do similar things the same way throughout the codebase.
Consistency reduces surprise and cognitive load
even when neither approach is objectively superior.
When a convention already exists, follow it;
do not introduce a competing convention
without compelling justification.

### Take a Rust-native approach (`rust-native`) {#rust-native}

Asterinas is inspired by Linux but is not a C port.
The language shapes how we think about problems:
where C code relies on conventions and manual discipline
(return-code checking, paired init/cleanup, header-file contracts),
Rust offers compiler-enforced, zero-cost abstractions
(the `?` operator, RAII, trait bounds).

Learn from Linux's design, not its idioms.
The result should read like idiomatic Rust,
not like C written in Rust syntax.
