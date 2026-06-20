# Layout

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
