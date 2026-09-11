# Interface and Output

## Command line

```text
tools/acr/run.sh [OPTIONS] diff  <base>             <output>
tools/acr/run.sh [OPTIONS] files <path[:lines] ...> <output>
```

| Argument | Meaning |
|---|---|
| `diff <base>` / `files <targets>` | What to review. See the two modes below. |
| `<output>` | Report path. Always the last positional argument. A relative path is resolved from the repository root. |
| `--overwrite` | Replace an existing report. Without it, ACR stops before rewriting. |
| `--per-persona-context=auto\|yes\|no` | Review personas separately or together. See [Persona contexts](#configuration). |
| `--backend=NAME` | `openai-agents`, `pi-agent`, `claude-agent`, or `fake`. |
| `--config PATH` | Profile to use. See [Configuration](#configuration). |

On success ACR prints the report path, the run log, and the trace file;
on error it prints `acr: <reason>` and exits with status 2.

Examples:

```sh
tools/acr/run.sh diff origin/main /tmp/review.md --overwrite
tools/acr/run.sh --config tools/acr/acr.pi.toml files kernel/src/fs/file.rs:40-120 /tmp/review.md
```

## Reviewing the current checkout

Both modes review the current checkout; there is no head argument.
To review another commit, branch, or tag, make it `HEAD` first,
for example with `git worktree add /tmp/old <ref>` and run ACR there.

### `diff <base>`: review commits

`diff <base>` reviews the commits in `merge-base(<base>, HEAD)..HEAD`, oldest first.
Each commit is shown with its message and its diff,
so ACR can judge whether each commit does what its message says
and whether the commits are well-formed,
not only the net code change.

- `<base>` is any Git revision: `main`, `origin/main`, a tag, or a SHA.
- ACR reviews only the commits added on your branch, not commits added to the base branch after you branched off.
- Only committed changes are reviewed.
  Commit uncommitted edits first, or review the files with `files`.
- If the range contains no commits, ACR fails instead of writing an empty review.
- Ranges such as `main..topic` are rejected; check out `topic` and run `diff main`.

### `files <path[:lines] ...>`: review files

`files` reviews the current working-tree contents of one or more files,
including uncommitted edits.
Each target is a path relative to the repository root,
optionally followed by 1-based, inclusive line ranges:

```text
kernel/src/sched/fair.rs                    # whole file
kernel/src/sched/fair.rs:120                # one line
kernel/src/sched/fair.rs:120-180            # one range
kernel/src/sched/fair.rs:120-180,240-260    # several ranges
```

A path may be repeated; its ranges are merged.
The selected lines are what ACR reports on,
but its agents may read the rest of the repository for context.

**Unusual paths.**
A colon followed by line numbers at the end of a path selects which lines to review,
for example foo.rs:10-20,30. Otherwise, the colon is part of the filename.

- A file whose name itself ends like a range, such as `data:12-20`,
  must be wrapped in double quotes that reach ACR: `'"data:12-20"'` in Bash.
  A range may follow the closing quote: `'"data:12-20":1-5'`.
- Put paths containing spaces in quotes, for example 'docs/a file.md'.
  ACR currently supports only whole-file reviews for these paths.

## Configuration

A run is configured by one TOML **profile**.
ACR reads only the selected profile.
Missing settings use built-in defaults,
not values from another profile.

Credentials are never stored in a profile.
The `openai-agents` backend reads the API key and endpoint from the environment variables
named by `provider.api_key_env` and `provider.base_url_env` respectively;
the `pi-agent` backend inherits the local Pi's own login or third-party config;
the `claude-agent` backend inherits the local Claude CLI's authentication and connection config.

The reference profiles are [`acr.toml`](../acr.toml) (`openai-agents`),
[`acr.pi.toml`](../acr.pi.toml) (`pi-agent`),
and [`acr.claude.toml`](../acr.claude.toml) (`claude-agent`).
The supported tables are:

| Table | Key settings |
|---|---|
| `[agent]` | `backend`; `review_model` for persona reviews and `postprocess_model` for verification, consolidation, summary, and benchmark grading; `reasoning_effort` for persona reviews and `postprocess_reasoning_effort` for later stages; `per_persona_context`; `max_concurrency`; `timeout_seconds` per agent attempt (unset means no limit); `retries` of a whole agent run; `model_retries` of a single model request (`openai-agents` only). |
| `[tools]` | One allowlist of tool names per agent role: the five personas, `verification`, `consolidation`, `summary`, and `grader`. |
| `[provider]` | `openai-agents` only: `adapter` (`openai` or `any-llm`), `api_key_env`, `base_url_env`. |
| `[benchmark]` | `remote`: where to fetch benchmark commits that are missing locally. |

**Model selection.**
`review_model` and `postprocess_model` are independent settings.
You can modify both in the profile.
The Claude reference profile sets both to `"default"` to inherit local Claude CLI's model.

**Persona contexts.**
`per_persona_context = "yes"` runs one agent per active persona, in parallel;
`"auto"` currently means the same and is the default.
`"no"` runs a single agent for all active personas:
cheaper, but it may find fewer defects
(see [Execution model](execution_model.md#one-agent-per-persona)).
Override it for one run with `--per-persona-context=...`.

**Web search.**
Adding `web.search` to a role's allowlist enables the provider's hosted web search
for that role on `openai-agents` backend.
It requires `wire_api = "responses"` and an endpoint that supports it.

## The report

The report is Markdown with YAML front matter,
a summary, and one section per persona that has findings:

````markdown
---
date: 2026-09-06
mode: files
files: kernel/src/example.rs:10-30
head: 1a2b3c4-dirty
branch: feature
---

# Summary

<overview of the findings>

## Correctness

### `kernel/src/example.rs` line 18

> ```diff
> +    let value = entries[index];
> ```

Bounds check missing (major): A user-controlled index can panic the kernel.

**Fix.** Validate the index before indexing and return `EINVAL`.

## Retracted by verification

- `c3-5f0e9a1b2c7d`: `index` is checked against `entries.len()` at line 12.
````

**Front matter.**
`mode`, then `base` (the merge-base) in `diff` mode or `files` (the normalized targets) in `files` mode,
then `head` and `branch`.
In files mode, head includes -dirty if the repository has uncommitted changes to tracked files,
even if those changes are outside the selected files.

**Sections** appear in a fixed order:
Maintainability, Correctness (the Development persona), Security, Hardware, Documentation.
Sections without findings are omitted.
If there are no findings at all, the report contains only the front matter and the summary.

**A finding** is headed by its file (or commit) and optional line, and contains:

- an optional quoted excerpt or minimal diff;
- the **grounding**: a guideline short-name in backticks,
  or a plain-language defect class when no rule applies;
- the severity: `critical` (must fix), `major` (should fix), `minor` (worth fixing), or `nit` (optional);
- the problem and its consequence;
- `**Fix.**` with a concrete remedy.

Each finding is validated against the `ReviewComment` schema in [`core/contracts.py`](../src/core/contracts.py)
before it can enter the report.

**Verification marks.**
A finding whose key claim could not be confirmed starts with `(unverified)`.
A finding shown to be false leaves its section
and is listed under `Retracted by verification` by its internal ID.
See [Execution model](execution_model.md#verification-consolidation-and-summary).
