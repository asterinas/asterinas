# Design

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
[`narrow-visibility`](rust-specific/crates-and-modules.md#narrow-visibility)
for Rust-specific visibility rules;
PR [#2951](https://github.com/asterinas/asterinas/pull/2951#discussion_r2786925410).

### Keep functions small and focused (`small-functions`) {#small-functions}

Each function should do one thing,
do it well, and do it only.
If you can extract another function from it
with a name that is not merely a restatement
of its implementation,
the original function is doing more than one thing.

Do not mix levels of abstraction.
For example, a syscall handler should read like a specification;
byte-level manipulation belongs in a helper.

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
