# Functions and Methods

### Minimize nesting (`minimize-nesting`) {#minimize-nesting}

Minimize nesting depth.
Code nested more than three levels deep
should be reviewed for refactoring opportunities.
Each nesting level multiplies the reader's cognitive load.

Techniques for flattening nesting:
- Early returns and guard clauses for error paths.
- `let...else` to collapse `if let` chains.
- The `?` operator for error propagation.
- `continue` to skip loop iterations.
- Extracting the nested body into a helper function.

The normal/expected code path
should be the first visible path;
error and edge cases
should be handled and dismissed early.

```rust
pub(crate) fn init() {
    let Some(framebuffer_arg) = boot_info().framebuffer_arg else {
        log::warn!("Framebuffer not found");
        return;
    };
    // ... main logic at the top level
}
```

See also:
PR [#2877](https://github.com/asterinas/asterinas/pull/2877#discussion_r2685861741).

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

### Avoid boolean arguments (`no-bool-args`) {#no-bool-args}

A boolean parameter that selects between
two behaviors signals the function does two things.
Split it into two functions
or use a typed enum.

```rust
// Good — two separate functions
fn read(&self, buf: &mut [u8]) -> Result<usize> { ... }
fn read_nonblocking(&self, buf: &mut [u8]) -> Result<usize> { ... }

// Good — typed enum
enum ReadMode { Blocking, NonBlocking }
fn read(&self, buf: &mut [u8], mode: ReadMode) -> Result<usize> { ... }

// Bad — boolean argument
fn read(&self, buf: &mut [u8], blocking: bool) -> Result<usize> { ... }
```

See also:
_Clean Code_, Chapter 3 "Flag Arguments".
