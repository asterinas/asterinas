# Testing

### Add regression tests for every bug fix (`add-regression-tests`) {#add-regression-tests}

When a bug is fixed,
a test that would have caught the bug should accompany the fix.
Record the issue reference in the commit message or pull request description.
[Do not include an issue link in code unless it adds useful context](../for-maintainability/comments.md#useful-issue-links).

See also:
PR [#2962](https://github.com/asterinas/asterinas/pull/2962).

### Test user-visible behavior, not internals (`test-visible-behavior`) {#test-visible-behavior}

Tests should validate observable, user-facing outcomes.
Prefer testing through public APIs
rather than exposing internal constants in test code.

Name tests after the behavior or specification concept being verified,
not after internal implementation details.
Using kernel-internal names in user-space regression tests
creates unnecessary coupling.

See also:
PR [#2926](https://github.com/asterinas/asterinas/pull/2926).

### Use assertion macros, not manual inspection (`use-assertions`) {#use-assertions}

Use language- or framework-provided assertion helpers
instead of printing values and manually inspecting output.
Assertions provide clear failure messages
and make tests self-checking.

For C regression tests, see
[which assertion macros to use in tests, setup, and child processes](path-specific/regression.md#test-assertion-context).

See also:
PR [#2877](https://github.com/asterinas/asterinas/pull/2877)
and [#2926](https://github.com/asterinas/asterinas/pull/2926).
