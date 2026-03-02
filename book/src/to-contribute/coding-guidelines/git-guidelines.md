# Git Guidelines

These guidelines cover commit hygiene and pull request conventions.
For the underlying philosophy, see
[How Guidelines Are Written](how-guidelines-are-written.md).

### Write imperative, descriptive subject lines (`imperative-subject`) {#imperative-subject}

Write commit messages in imperative mood
with the subject line at or below 72 characters.
Wrap identifiers in backticks.

Common prefixes used in the Asterinas commit log:

- `Fix` — correct a bug
- `Add` — introduce new functionality
- `Remove` — delete code or features
- `Refactor` — restructure without changing behavior
- `Rename` — change names of files, modules, or symbols
- `Implement` — add a new subsystem or feature
- `Enable` — turn on a previously disabled capability
- `Clean up` — minor tidying without functional change
- `Bump` — update a dependency version

Examples:

```
Fix deadlock in `Vmar::protect` when holding the page table lock

Add initial support for the io_uring subsystem

Refactor `TcpSocket` to separate connection state from I/O logic
```

If the commit requires further explanation,
add a blank line after the subject
followed by a body paragraph
describing the _why_ behind the change.

See also:
PR [#2877](https://github.com/asterinas/asterinas/pull/2877)
and [#2700](https://github.com/asterinas/asterinas/pull/2700).

### One logical change per commit (`atomic-commits`) {#atomic-commits}

Each commit should represent one logical change.
Do not mix unrelated changes in a single commit.
When fixing an issue discovered during review
on a local or private branch,
use `git rebase -i` to amend the commit
that introduced the issue
rather than appending a fixup commit at the end.

See also:
PR [#2791](https://github.com/asterinas/asterinas/pull/2791)
and [#2260](https://github.com/asterinas/asterinas/pull/2260).

### Separate refactoring from features (`refactor-then-feature`) {#refactor-then-feature}

If a feature requires preparatory refactoring,
put the refactoring in its own commit(s)
before the feature commit.
This makes each commit easier to review and bisect.

See also:
PR [#2877](https://github.com/asterinas/asterinas/pull/2877).

### Keep pull requests focused (`focused-prs`) {#focused-prs}

Keep pull requests focused on a single topic.
A PR that mixes a bug fix, a refactoring,
and a new feature is difficult to review.

Ensure that CI passes before requesting review.
If CI fails on an unrelated flake,
note it in the PR description.
