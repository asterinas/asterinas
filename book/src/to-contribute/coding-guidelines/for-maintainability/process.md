# Process

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

### Explain the motivation and changes in PR summaries (`pr-summary`) {#pr-summary}

Start the PR description with the concrete problem or need
and the resulting change.
Explain the key approach and why it addresses the problem.
Give readers enough context to understand the change
without reconstructing its purpose from the diff or earlier discussions.

Tailor the supporting details to the change:

- **Bug fixes:** Explain the trigger, incorrect behavior, root cause,
  and why the fix works; note the regression test coverage.
- **Features:** State the use case, capability delivered,
  and any prerequisites or incomplete support.
- **Performance changes:** Identify the bottleneck
  and provide relevant before-and-after measurements,
  with the workload and configuration.
- **Refactoring and maintenance:** Explain the concrete benefit
  and identify any behavior changes.

Link directly related issues, proposals, and discussions,
and summarize their relevant conclusions.
A link or `Fixes #1234` alone is not an explanation.
Use `Fixes` or `Closes` only when the PR resolves the issue;
identify the remaining scope for partial implementations.

Match the detail to the change.
One or two sentences can suffice for a small cleanup.
Omit file-by-file inventories, development history,
empty template sections, and routine CI logs.
Update the description when review changes the scope or approach.
Disclose known limitations, costs, and validation gaps
that affect use or review,
and distinguish observed results from expectations.

**Optional:** Add examples, diagrams, alternatives considered,
review order, follow-up links, or a separate testing section
when they help explain or assess the change.

See also:
[Writing good CL descriptions](https://google.github.io/eng-practices/review/developer/cl-descriptions.html);
[Linux: Describe your changes](https://docs.kernel.org/process/submitting-patches.html#describe-your-changes).

### Discuss substantial designs before implementation (`design-before-large-changes`) {#design-before-large-changes}

For a large feature or major architectural change,
first check whether it requires the [RFC process](../../../rfcs/0001-rfc-process.md#when-to-follow-the-rfc-process).
For changes requiring an RFC, obtain RFC approval before substantial implementation.
For other substantial designs, open or reuse a design proposal issue
and discuss the goals and approach with the relevant maintainers
before substantial implementation work.
Judge by design impact, not line count:
new subsystems, new architecture support, core OSTD abstractions,
and changes to ownership, synchronization, or safety boundaries
across subsystems warrant a proposal.
Routine fixes, mechanical edits, and local additions under an existing design
do not require a new proposal.

The proposal should cover the problem, goals, overall design,
key trade-offs, affected components, and unresolved questions.
For complex features, include an implementation breakdown
and validation approach.

Link the approved RFC or design proposal issue from the implementation PR
and summarize the motivation, key changes, and scope of this step.
Call out material departures from the proposal and outstanding decisions.
A series of PRs can share one proposal.

See also:
[PyTorch: Common Mistakes To Avoid](https://docs.pytorch.org/docs/main/community/contribution_guide.html#common-mistakes-to-avoid);
[design proposal #3295](https://github.com/asterinas/asterinas/issues/3295)
and its [implementation PR #3421](https://github.com/asterinas/asterinas/pull/3421).
