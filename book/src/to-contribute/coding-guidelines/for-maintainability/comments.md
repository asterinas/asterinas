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

For platform differences in C regression tests, see
[how to explain the differing assertions in FIXME comments](../for-development/path-specific/regression.md#explain-test-differences).

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

### Do not include issue links unless they add useful context (`useful-issue-links`) {#useful-issue-links}

Do not put issue numbers or issue links in code by default,
including unit tests and regression tests.
Labels such as `Fix for #4081` or `Regression test for #4081`
only record the code's history;
keep those references in the commit message or pull request description.
Names and comments should describe the code's behavior.

An exception is an issue that adds useful information for understanding the code,
such as an analysis of how an application depends on the behavior.
When linking it, explain in the comment what the reader will learn
and why it matters to this code.
Describe the most relevant or important behavior in the comment itself
so readers can understand the code without opening the issue.

```c
// Good — explains why the linked analysis is relevant
// With a zero wake count, FUTEX_REQUEUE must move the waiter without waking it.
// musl uses this when transferring a condition-variable waiter to a mutex.
// For an analysis of this musl path, see
// <https://github.com/asterinas/asterinas/issues/3964>.

// Bad — records history without explaining what the link adds
// Fix for #3964.
// Regression test for #3964.
```

See also:
PR [#4083](https://github.com/asterinas/asterinas/pull/4083#discussion_r4476299073)
and [#4135](https://github.com/asterinas/asterinas/pull/4135#discussion_r4440242488).
