# Interface and Output

This is the reference for everything a user of ACR touches:
the command line, the two review modes, configuration, and the report.

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
| `--per-persona-context=auto\|yes\|no` | Review personas separately or together; overrides `agent.per_persona_context`. See [Profile settings](#profile-settings). |
| `--config PATH` | Profile to use, which also selects the backend. Default: `acr.openai.toml`. See [Choosing a profile](#choosing-a-profile). |

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

A run is configured in four layers.
For each setting, the first layer that provides it wins:

```text
command-line flag  >  ACR_* environment variable  >  TOML profile  >  built-in default
```

Flags are for a single run, environment variables for a shell session or a script,
and the profile for choices you keep.
Internal policy, such as which tools each agent may use, is built in and not configurable
(see [Read-only tools](execution_model.md#read-only-tools)).

### Choosing a profile

A **profile** is one TOML file, and it alone selects the backend:
its models and backend table only make sense for the backend they were written for,
so `agent.backend` has no command-line flag or environment variable.
ACR reads exactly one profile and never merges several:
the file given by `--config PATH`, which must exist,
or [`acr.openai.toml`](../acr.openai.toml) by default.

| Backend | Reference profile |
|---|---|
| `openai-agents` | [`acr.openai.toml`](../acr.openai.toml) |
| `pi-agent` | [`acr.pi.toml`](../acr.pi.toml) |
| `claude-agent` | [`acr.claude.toml`](../acr.claude.toml) |
| `fake` | [`tests/acr.fake.toml`](../tests/acr.fake.toml): no model, empty findings; checks the pipeline only. |

To customize a run, copy a reference profile, edit it, and pass it with `--config`.
Missing settings use built-in defaults, not values from another profile.

### Profile settings

```toml
[agent]                       # settings shared by all agents
backend = "openai-agents"
per_persona_context = "auto"
# log_root = "/tmp/aster-code-review"

[agent.persona_review]        # persona reviews, separate or combined
model = "..."
effort = "medium"

[agent.postprocess]           # verification, consolidation, summary, benchmark grading
model = "..."
effort = "medium"

[openai]                      # read only by the openai-agents backend
adapter = "any-llm"
wire_api = "responses"
```

| Key | Meaning | Default |
|---|---|---|
| `agent.backend` | `openai-agents`, `pi-agent`, `claude-agent`, or `fake`. | `openai-agents` |
| `agent.per_persona_context` | `yes` runs one agent per active persona, in parallel; `auto` currently means the same. `no` runs one combined agent: cheaper, but it may find fewer defects (see [Execution model](execution_model.md#one-agent-per-persona)). | `auto` |
| `agent.log_root` | Directory for [run records](execution_model.md#run-records). | `$TMPDIR/aster-code-review` |
| `agent.<stage>.model` | Model for the stage. Its format depends on the backend; see the backend's document. | `gpt-5.5` |
| `agent.<stage>.effort` | Reasoning effort for the stage: `low`, `medium`, `high`, or `max`. | `high` for `persona_review`, `low` for `postprocess` |
| `openai.adapter` | `openai` for the official OpenAI API, `any-llm` for third-party relays. | `openai` |
| `openai.wire_api` | `responses` or `chat_completions`. | `responses` |

The two stages are independent: changing one stage's model or effort never changes the other.
A backend reads only its own table; the [Pi backend](pi_backend.md#how-to-use-this-backend) has an optional `[pi]` table.
An unknown key is an error, so a typo cannot be silently ignored.
Concurrency, retries, and time limits are built in, not settings
(see [Run limits](execution_model.md#run-limits)).

### Environment variables

Every profile key except `agent.backend` can be overridden by an environment variable named
`ACR_` + the key without `agent.`, upper-cased, with dots replaced by underscores:

| Profile key | Environment variable |
|---|---|
| `agent.per_persona_context` | `ACR_PER_PERSONA_CONTEXT` |
| `agent.persona_review.model` | `ACR_PERSONA_REVIEW_MODEL` |
| `agent.postprocess.effort` | `ACR_POSTPROCESS_EFFORT` |
| `openai.wire_api` | `ACR_OPENAI_WIRE_API` |

A few variables control the process rather than a profile setting:

| Variable | Effect |
|---|---|
| `ACR_PYTHON` | Python interpreter used by `run.sh`. Default: `python3`. |
| `ACR_API_KEY`, `ACR_BASE_URL` | API key and endpoint for `openai-agents` (see [Credentials](#credentials)). |
| `ACR_GUIDELINE_ROOT` | Copy of the Coding Guidelines to review against (see [Coding guidelines](coding_guidelines.md#which-copy-of-the-guidelines-is-used)). |
| `ACR_GUIDELINE_DISCLOSURE` | `full` inlines every rule instead of loading rules on demand (see [Coding guidelines](coding_guidelines.md#loading-rules-on-demand)). |

### Credentials

Credentials are never stored in a profile, and ACR checks them itself before calling a model:

- `openai-agents` reads the API key from `ACR_API_KEY` and the endpoint from `ACR_BASE_URL`,
  and fails if the key is unset.
  An unset endpoint means the official OpenAI API.
  The names are fixed; if your key is stored elsewhere, copy it, e.g. `export ACR_API_KEY="$MY_KEY"`.
- `pi-agent` uses Pi's own login and provider configuration.
- `claude-agent` uses the local Claude CLI's authentication and connection settings.

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

- a quoted excerpt or minimal diff (commit-message text for a commit-message finding);
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
