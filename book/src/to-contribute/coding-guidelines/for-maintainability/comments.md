# Comments

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

When code handles a platform difference, explain the behavior on each platform
and why it requires different code or test assertions.
If the difference requires a temporary workaround,
also state what must change before the workaround can be removed:

```c
// Good — explains both outcomes and the assertion they affect
/*
 * FIXME: Linux makes /proc/self/fd root-owned when dumpable is cleared.
 * Asterinas does not support dumpable and uses the effective UID instead.
 * Check root ownership only on Linux until Asterinas supports dumpable.
 */

// Bad — leaves the reader to reconstruct the difference
// FIXME: Procfs ownership differs between Linux and Asterinas.
```

See also:
Linux man-pages [`proc_pid(5)`](https://man7.org/linux/man-pages/man5/proc_pid.5.html);
PR [#2265](https://github.com/asterinas/asterinas/pull/2265#discussion_r2266220943)
and [#2050](https://github.com/asterinas/asterinas/pull/2050#discussion_r2224106025);
the platform-difference comment in
[`proc_fd_open_fifo_after_setid.c`](https://github.com/asterinas/asterinas/blob/5f41ff02d681e047f9420bb18c162cfe76116606/test/initramfs/src/regression/fs/procfs/proc_fd_open_fifo_after_setid.c).

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
