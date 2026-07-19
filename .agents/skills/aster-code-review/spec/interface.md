# Interface & Output

## How the skill is invoked

`aster-code-review` is a **skill, not a binary**.
You trigger it from inside a Claude Code or Codex session;
the host agent then runs the orchestration in [`execution_model.md`](execution_model.md).
There is no command on your `PATH` — though the skill also ships a headless CLI (below) for running it from a plain shell or from CI.
However it is triggered, the skill understands one small argument string,
and that string begins with a **mode word**:

```
diff   <base>              <output> [--overwrite] [--per-persona-context=auto|yes|no]
files  <path[:lines] ...>  <output> [--overwrite] [--per-persona-context=auto|yes|no]
```

- **`diff`** — *review a Git change*:
  the defects in what the working tree adds over a `<base>`.
- **`files`** — *review target code*:
  the defects in a set of files,
  optionally narrowed to line ranges.

Both modes write the same kind of review file (below) and run the same persona fan-out (see [`execution_model.md`](execution_model.md));
they differ only in *what code they put under review*.

### Running it headless — the `aster_code_review.sh` CLI

Usually the skill runs inside an agent session.
It also ships a headless CLI at the skill root, `aster_code_review.sh`,
that runs the whole skill from a plain shell by driving an agent for you:

```
ACR_AGENT_PROFILE=<name> aster_code_review.sh <mode> <args…> <output> [flags]
```

The arguments **are** the argument string above,
so the CLI is identical to an in-session invocation;
the only addition is the agent, chosen by the environment rather than an argument
(a given environment exposes only a few agents):

- `ACR_AGENT_PROFILE` (required) — the agent profile to run under
  (see [`benchmark.md`](benchmark.md#agent-profiles) for the profile format and the shipped `claude` / `codex` / `codex_workflow` profiles).
- `ACR_PROFILE_VARIANT` (optional) — `smoke` for the low-effort overlay.

It reviews the current working tree, so `cd` into the repo (or a scratch worktree) first.
Under the hood it builds the invocation prompt and hands it to the shared launcher `scripts/run_agent.sh`,
which is the one thing that turns a profile into a running agent.
The benchmark and the PR-review CI both drive the skill through this one CLI.

## The rule: HEAD is the head — there is no head argument

Whichever mode you use, the skill reviews relative to the **current checkout (HEAD)**;
there is never a "head" or "right endpoint" argument.
`diff <base>` reviews the **commits** the branch added to reach HEAD
— the series `merge-base(<base>, HEAD)..HEAD`;
`files` reviews the **working-tree contents** of the named files at HEAD.
To review at a specific commit,
branch, or tag, **make it HEAD**
— `git checkout <ref>`, or `git worktree add` a scratch tree and `cd` into it
— then run the skill.

This keeps the interface small and gives one mental model across PR review,
periodic quality scans, and the self-review loop:

- no head/range bookkeeping — you name a base (or some files), never two endpoints;
- `diff` reviews **committed** commits,
  so commit your work first — the self-review loop commits each round,
  making its work part of the series;
  uncommitted edits are not in a diff review (review them with `files`, or commit them);
- `files` reviews the working tree as it stands,
  so it *does* cover uncommitted edits to the files you name.

The earlier `<base..head>` range is gone:
an explicit head endpoint contradicts "HEAD is the head."
Reviewing an arbitrary historical range means checking that range's head out first.

## Mode 1 — `diff <base>`: review a Git change

`diff <base>` reviews the **commit series** the branch added over its merge-base with `<base>`
— the commits `merge-base(<base>, HEAD)..HEAD`,
oldest first, each presented as its **message** and its **diff**:

```
git log -p --reverse $(git merge-base <base> HEAD)..HEAD
```

Reviewing the series rather than one flattened diff is what lets the skill judge **per-commit intent** (does each commit do what its message claims?)
and **commit hygiene** (imperative subject, atomic commits — a Maintainability concern),
on top of the code itself.

- On a clean branch this is *what the branch added since it diverged from `<base>`*
  — the pull request.
  `<base>` may be any ref or SHA (`main`, `origin/main`, a tag, a short SHA).
- Using the *merge-base* (not `<base>` directly) means a `<base>` that advanced after you branched does not inject its upstream commits as spurious reversals
  — only the branch's own commits are reviewed.
- **Committed commits only.**
  Uncommitted edits, and brand-new untracked files,
  are not in a diff review — commit them first (the self-review loop commits each round),
  or review a specific file with `files <path>`.

## Mode 2 — `files <path[:lines] ...>`: review target code

`files` reviews the working-tree contents of the files you name,
optionally narrowed to line ranges.
It is for auditing existing code rather than a change
— periodically scanning select files to raise quality,
or focusing a review on one area.

Each **target** is a path, optionally followed by line ranges:

```
TARGET := PATH | PATH ":" RANGES | '"' QPATH '"' | '"' QPATH '"' ":" RANGES
RANGES := RANGE ("," RANGE)*
RANGE  := N | N "-" M           # 1-based, inclusive, N ≤ M
```

- `kernel/core/src/sched/sched_class/fair.rs` — review the whole file.
- `kernel/core/src/sched/sched_class/fair.rs:120-180` — review only lines 120–180.
- `kernel/core/src/sched/sched_class/fair.rs:120-180,240-260` — two ranges in one file.
- Repeating a path is allowed; its ranges are unioned, and overlapping ranges are merged.

The named lines are the **review scope**;
the rest of the file and the wider repository remain readable as **context**
— exactly as in `diff` mode,
where the diff is the unit of review but a pass may read around it.

### Quoting paths

A path that contains a space or a colon
— or whose final segment *looks like* a line range
— would confuse the bare grammar.
Wrap it in double quotes:

- `"a path with spaces.rs"` — whole file, spaces and all.
- `"weird: dir/mod.rs":30-60` — colon in the path, plus a range.
- `"gen/12-20"` — a file literally named `12-20`,
  reviewed whole (quotes stop it being read as `path:range`).

Unquoted, the parser splits the line spec **from the right**:
it peels a trailing `:RANGES` only if the suffix fully matches the range grammar,
otherwise the whole token is the path.
So `weird:dir/mod.rs` — whose tail is not a range
— is already a valid bare path;
quotes are the explicit escape for the genuinely ambiguous rest.

## How the argument string is parsed

The skill receives its arguments as **one raw string** and tokenizes it itself,
deterministically — it does **not** rely on a shell to do so.
When you type `/aster-code-review files "a b.rs" out.md` in Claude Code,
the prompt is not a POSIX shell:
the quotes arrive intact, so the skill must own the parse.
The grammar:

1. The first token is the **mode word**, `diff` or `files`.
2. The remainder is split into tokens on whitespace,
   **except inside double quotes**
   — a quoted span is one token, spaces and colons included.
3. **Flags** (`--overwrite`) are pulled out.
4. The **last** positional is `<output>`;
   everything before it is the target list
   — one `<base>` for `diff`, one or more targets for `files`.
5. In `files` mode each target is parsed by the grammar above:
   quotes strip to the bare path,
   and an unquoted trailing `:RANGES` is split from the right.

Owning the parse — rather than leaning on the agent or a shell to pre-split the string
— makes it identical no matter how the skill was triggered,
and keeps it a deterministic step the benchmark exercises directly.

### The argument table

| Argument | Meaning |
|---|---|
| `diff` \| `files` | **Required, first.** The review mode. |
| `<base>` | (`diff`) **Required.** Any ref or SHA; the review covers the commit series `merge-base(<base>, HEAD)..HEAD` — each commit's message + diff. |
| `<path[:lines] ...>` | (`files`) **Required, one or more.** The files (and optional line ranges) to review, in the working tree. |
| `<output>` | **Required, last.** Where to write the review; refuses to clobber an existing file unless `--overwrite` is given. |
| `--overwrite` | Replace the output file if it already exists. |
| `--per-persona-context` | `yes` fans out one isolated agent per persona (best recall); `no` runs one combined agent over all personas (cheaper, lower recall); `auto` (default) currently resolves to `yes`. Affects orchestration only — see [`execution_model.md`](execution_model.md). |

### Rationale for the argument shape

- **A leading mode word, not a guessed argument.**
  The mode is named explicitly because the alternative
  — detecting whether the first token is a ref or a path
  — is genuinely ambiguous (`main` vs `main.rs`, a ref that is also a filename, a path absent at HEAD) and against this interface's no-magic ethos.
  One extra word buys an unambiguous,
  self-documenting parse and room for a third mode later.
- **Output is the last positional, `cp`-style.**
  With a variadic target list in `files` mode,
  the output cannot live in a fixed second slot.
  "Last positional is the destination" is the familiar `cp src... dest` convention and keeps both modes' grammars uniform.
  It stays **mandatory** — there is no silent default review file,
  which keeps the benchmark deterministic and never surprises a human with a file they did not name.
- **HEAD is the head, so no head argument.**
  There is never a second endpoint:
  `diff` reviews the commit series up to HEAD,
  `files` the working tree at HEAD.
  This replaces the old `<base..head>` range and the removed `--worktree` flag.
  The cost is that reviewing an arbitrary historical range means checking its head out first;
  the gain is one consistent model across PR review,
  scans, and the self-review loop (which commits each round).
- **Quoting is the skill's, not the shell's.** Because the trigger is not a shell,
  the skill self-tokenizes and treats double quotes as its own grammar
  — the only robust way to let a path hold spaces or colons given that the quotes survive into the raw argument string.
- **An existing review is never clobbered by accident.**
  Overwriting is opt-in via `--overwrite`:
  a loop that means to replace its previous review passes the flag;
  a hand-annotated review stays safe by default.
- **Cost is an explicit, opt-in knob.**
  `--per-persona-context=no` collapses the five-way persona fan-out into one combined pass
  — cheaper, but with measurably lower recall (a single context dilutes focus; see [`execution_model.md`](execution_model.md)).
  It is never the silent default:
  `auto` resolves to the recall-first fan-out today and will graduate to a benchmark-driven heuristic.
  Recall-first means the cheap path is something a caller asks for,
  not something the skill does behind your back.

## The review file

The output is one Markdown file:
YAML frontmatter, a constructive `# Summary`,
then a `## <Persona>` section per activated persona,
each holding one `### <location>` subsection per comment.

```markdown
---
date: 2026-06-28
mode: diff
base: a1b2c3d
head: 9f3a1c2
branch: my-feature
title: "Report real RSP before the trap in TrapFrame"
---

# Summary

<What the change does well; the top issues that need attention, ranked by
severity; any structural recommendations.>

## Maintainability

### `ostd/src/arch/x86/trap/mod.rs` line 12

> ```diff
> +    let n = frame.size();
> ```

`descriptive-names` (nit): `n` is opaque at the point of use.

**Fix.** Rename it to `frame_size` so the call site reads on its own.

## Hardware

### `ostd/src/arch/x86/trap/mod.rs` line 30

> ```diff
> -    _pad: usize,
> ```

`16b-align-rsp-before-call` (critical): removing `_pad` makes
`size_of::<TrapFrame>()` no longer a multiple of 16, but the CPU aligns `%rsp`
to 16 bytes on trap entry (System V AMD64 ABI). An odd-sized frame misaligns the
stack and is unsound for SSE.

**Fix.** Restore the padding and add a
`const_assert!(size_of::<TrapFrame>().is_multiple_of(16))`.
```

The frontmatter records the **`mode`** and what was reviewed:

- **`diff`** records the `base` (the merge-base) and the `head` (HEAD's short SHA).
  It reviews committed commits, so `head` is never `-dirty`.
- **`files`** records the reviewed `files` (with any ranges) instead of a `base`;
  `head` is HEAD's short SHA, suffixed `-dirty` when the working tree has uncommitted edits (files mode reviews the working tree).

The rest — `date`, `branch`, optional `title` — is common to both modes.

## The comment model

Comments are **grouped under their persona's `##` section**,
headed by that persona's review concern
— **Maintainability**, **Correctness**,
**Security**, **Hardware**, **Documentation**
— rather than carrying a per-comment tag.
(The Development persona's section is titled **Correctness**: a bare `## Development` would read oddly next to the others, since all of this is development.)
Each comment is a `###` subsection that carries:

- a **location** heading — `` `path` line N `` (or `lines N-M`),
  or a commit locus like `` `commit abc1234 message` `` for a finding about a commit message;
- a quoted **snippet** of the relevant lines
  — a `diff` hunk (`diff` mode),
  the source lines (`files` mode),
  or the offending part of a commit message;
- a prose body opening with the grounding and severity — `` `<short-name>` (<severity>): ``
  for a guideline, `<description> (<severity>):` for a bug —
  then a statement of the problem, constructive and specific;
- a separate **Fix.** paragraph (see below).

The **grounding** is one of two kinds
— and this split is the heart of the subjective-plus-objective stance from [`motivation.md`](motivation.md):

- **guideline-backed** — the rule short-name it rests on (a lowercase kebab identifier,
  e.g. `validate-at-boundaries`), rendered as code —
  making a *subjective* call traceable to the shared standard rather than an opinion;
- **bug-described** — for an undeniable defect (logic error, UB, data race) that no rule covers,
  a short plain-language description of the defect (e.g. "Off by one", "Use after free"), rendered as prose.
  A *description*, not a coined short-name — so it never reads as a rule — and never the bare word `bug`.
  This is a **first-class, encouraged** category:
  a pass must hunt bugs by *reasoning*,
  never suppress one because "no rule covers it".
  Requiring a citation must never silence a real defect that no rule happens to name.

The **severity** is `critical`,
`major`, `minor`, or `nit` (`critical`: must fix; `major`: should fix; `minor`: worth fixing; `nit`: optional or stylistic).
It is the persona's judgement,
used to rank the Summary's top issues
— and is *not* used to gate benchmark recall.

**Every comment must propose a remedy.**
Pointing out a problem is not enough
— a comment that flags an issue with no path to resolving it is incomplete.
So each comment closes with a separate paragraph led by **Fix.**,
giving a concrete change or snippet a human reviewer or the Ralph loop can apply directly.

Two later orchestrator passes may revise a comment after it is written:
**verification** can flag it `(unverified)` or retract it,
and **consolidation** can repoint its **Fix.** at a shared remedy.
Both are described in [`execution_model.md`](execution_model.md).

## Two consumers

The review file is deliberately a plain Markdown artifact so it serves both audiences without adaptation:

1. **PR comments.**
   Each comment is file+line anchored,
   so a thin adapter posts the comments as inline review comments on a pull request:
   [`scripts/post_reviews_to_github.sh`](../scripts/post_reviews_to_github.sh) parses a review file and posts it
   (dropping any comment whose line is not on the PR diff, which GitHub would silently discard).
   The workflow `.github/workflows/review_pr_with_codex.yml`, triggered by an `/aster-code-review` PR comment,
   drives exactly this: run the skill via the CLI, then post with the adapter.
2. **The Ralph loop.**
   The agent reads the review file and fixes the flagged issues,
   then re-runs — the review is the loop's feedback signal.
   The per-comment **Fix.** paragraph is what makes this directly actionable.
   The loop **commits each round** and runs `diff <base>`,
   so the review covers its commits (per-commit intent and hygiene included);
   a quick edit it has not committed can still be reviewed with `files`.

### CI commands

On GitHub, a member triggers the skill on a PR with a `/aster-code-review` comment;
the CI mirrors the CLI/skill interface:

| Comment | Runs | Posts |
|---|---|---|
| `/aster-code-review` (or `… diff`) | review the PR diff | inline review comments |
| `/aster-code-review files <p1> … <pN>` | review those files (tracked; no spaces in a path, v1) | inline review comments |
| `/aster-code-review smoke [--problems="0002 0006"]` | the smoke test — does the skill run? | a ✅/❌ status comment |
| `/aster-code-review benchmark [--problems="0002 0006"]` | the benchmark — recall (informational) | a `recall X/Y` status comment |

`smoke`/`benchmark` run the **PR's** skill + guidelines against the **PR's** `problems.yaml`,
so a PR that adds a guideline and a fixture problem is verified end-to-end.
The comment is parsed by [`scripts/parse_pr_command.sh`](../scripts/parse_pr_command.sh) (strict allowlist),
and the workflow re-validates every value before it reaches a command.

### The CI contract

The PR-review workflow lives on the repository's default branch but runs the **PR's** copy of the skill
— a PR ships its own guidelines, so the review evolves with the code.
For that to stay robust as the skill changes,
the workflow depends on a **small, fixed surface** and nothing else:

- the profile name **`codex_workflow`**;
- **`aster_code_review.sh <mode> <args…> <output>`** — the argument grammar above, with `ACR_AGENT_PROFILE` in the environment;
- **`post_reviews_to_github.sh --repo <r> --pr <n> --head-sha <sha> [--finalize] <review-file>`**;
- **`scripts/parse_pr_command.sh`** (a `/aster-code-review …` comment → a validated plan) and the **`make smoke` / `make benchmark`** targets.

Everything else — the prompt wording, the `--per-persona-context` policy, the persona set,
the review-file format, and the coding guidelines —
is internal to the skill and free to change without touching the workflow.
