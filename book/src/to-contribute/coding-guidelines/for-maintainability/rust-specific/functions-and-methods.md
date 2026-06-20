# Functions & Methods

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

### Use block expressions to scope temporary state (`block-expressions`) {#block-expressions}

Use block expressions
when temporary variables are only needed
to produce one final value.
This keeps temporary state local
and avoids leaking one-off names into outer scope.

```rust
// Good — intermediate values are scoped to the block
let socket_addr = {
    let bytes = read_bytes_from_user(addr, len as usize)?;
    parse_socket_addr(&bytes)?
};
connect(socket_addr)?;

// Bad — temporary variables leak into outer scope
let bytes = read_bytes_from_user(addr, len as usize)?;
let socket_addr = parse_socket_addr(&bytes)?;
connect(socket_addr)?;
```

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
        warn!("Framebuffer not found");
        return;
    };
    // ... main logic at the top level
}
```

See also:
PR [#2877](https://github.com/asterinas/asterinas/pull/2877#discussion_r2685861741).

### Introduce explaining variables (`explain-variables`) {#explain-variables}

Break down complex expressions
by assigning intermediate results to well-named variables.
An explaining variable turns an opaque expression
into self-documenting code:

```rust
// Good — intent is clear
let is_page_aligned = addr % PAGE_SIZE == 0;
let is_within_range = addr < max_addr;
debug_assert!(is_page_aligned && is_within_range);

// Bad — reader must parse the whole expression
debug_assert!(addr % PAGE_SIZE == 0 && addr < max_addr);
```

See also:
_The Art of Readable Code_, Chapter 8 "Breaking Down Giant Expressions";
PR [#2083](https://github.com/asterinas/asterinas/pull/2083#discussion_r2512772091)
and [#643](https://github.com/asterinas/asterinas/pull/643#discussion_r1497243812).
