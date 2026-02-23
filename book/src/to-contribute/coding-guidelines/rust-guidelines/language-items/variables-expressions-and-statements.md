# Variables, Expressions, and Statements

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

### Use checked or saturating arithmetic (`checked-arithmetic`) {#checked-arithmetic}

Use checked or saturating arithmetic
for operations that could overflow.
Prefer explicit overflow handling
over silent wrapping:

```rust
// Good — overflow is handled explicitly
let total = base.checked_add(offset)
    .ok_or(Error::new(Errno::EOVERFLOW))?;

// Good — clamps instead of wrapping
let remaining = budget.saturating_sub(cost);

// Bad — may silently wrap in release builds
let total = base + offset;
```

If wraparound behavior is intentional,
use explicit `wrapping_*` or `overflowing_*` operations
and document why wrapping is correct.
