# Layout

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

In a test, use blank lines to separate setup, the operations being tested, and cleanup.
Keep each operation together with the assertions that check its result.
Separate independent cases with blank lines as well,
so readers can see where one case ends and the next begins.

See also:
PR [#4090](https://github.com/asterinas/asterinas/pull/4090#discussion_r4439266461).
