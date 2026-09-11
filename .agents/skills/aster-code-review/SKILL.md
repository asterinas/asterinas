---
name: aster-code-review
description: Review Asterinas commits or working-tree files with tools/acr using the OpenAI, Pi, or Claude backend, and summarize its Markdown report.
---

# Aster Code Review

Run ACR in the Asterinas development container and summarize its local Markdown report.
Reuse choices already supplied by the user; ask only for missing or ambiguous information.
Wait for required answers before proceeding.
Do not edit reviewed code.

## 1. Select the backend

Honor the user's explicit backend or profile.
Otherwise, select by the client running this skill, not by its model or installed CLIs:

| Client | Backend | Profile in `tools/acr/` |
|---|---|---|
| Pi | `pi-agent` | `acr.pi.toml` |
| Claude Code | `claude-agent` | `acr.claude.toml` |
| Codex | `openai-agents` | `acr.toml` |

The profile selects the backend, models, and tools; pass it with `--config`.
If neither the backend nor the client is known, ask the user to choose.

## 2. Use backend authentication

Use credentials for the chosen backend:

- **OpenAI:** Check for non-empty `OPENAI_API_KEY` and `OPENAI_BASE_URL` in ACR's environment.
  Report only set/unset status; do not obtain credentials from this chat agent's provider configuration.
  See [OpenAI setup](../../../tools/acr/spec/openai_agents_backend.md#how-to-use-this-backend) for details.
- **Pi:** Reuse Pi's existing login and provider configuration in `~/.pi/agent`.
  ACR's `review_model` and `postprocess_model` must resolve in Pi;
  use `provider/model-id` to disambiguate providers.
  See [Pi setup](../../../tools/acr/spec/pi_backend.md#how-to-use-this-backend) for details.
- **Claude:** ACR inherits the local Claude CLI's authentication and endpoint settings.
  For `review_model` and `postprocess_model`, `"default"` inherits the CLI model;
  explicit profile model names override it.
  See [Claude setup](../../../tools/acr/spec/claude_backend.md#how-to-use-this-backend) for details.

Pi and Claude require no OpenAI credentials.
If authentication fails, help the user configure credentials or log in to the selected backend;
do not silently switch backends or models.
For new OpenAI credentials, use secure input scoped to the ACR process,
or ask the user to export them locally before continuing.
Never request secrets in chat, print them, write them into files, or change global settings.

## 3. Resolve the review scope

Choose mode from the request; ask if unclear:

| Mode | Scope |
|---|---|
| `diff <base>` | Committed changes and commit messages in `merge-base(base, HEAD)..HEAD`. Excludes uncommitted edits. |
| `files <path[:lines] ...>` | Current file contents, including uncommitted edits. Paths are repository-relative; line ranges are 1-based and inclusive, e.g. `kernel/src/lib.rs:10-80`. |

For a single ordinary commit, review a checkout with that commit as `HEAD` and its parent as `base`.
For `base..head`, review a checkout at `head` and pass only `base` to ACR.
For a single merge or root commit, ask for a suitable range or file scope instead.

If asked to review uncommitted changes, identify targets with `git status --short`.
Exclude unrelated logs and generated files; clarify the file list only when the intended scope is ambiguous.

## 4. Run and report

Run from the reviewed checkout's root.
If a different `HEAD` is needed, create a temporary detached worktree;
use absolute paths to the original checkout's ACR runner and selected profile,
and keep the report outside that worktree.
Do not switch branches or alter the user's working tree.

Use the user's report path, or create a fresh one:

```sh
report="$(mktemp -d /tmp/acr-review.XXXXXX)/review.md"
```

Run one command for the chosen mode, using the selected profile (Pi shown below).
Here `base` is the resolved diff base; `files` is a shell array of selected `path[:lines]` targets:

```bash
tools/acr/run.sh --config tools/acr/acr.pi.toml diff "$base" "$report"
# Or, for files mode:
tools/acr/run.sh --config tools/acr/acr.pi.toml files "${files[@]}" "$report"
```

Use `--overwrite` only when replacing the existing report is authorized.
Wait for completion and check the exit status:

- **Success:** Read the report and summarize findings with location, severity, and suggested fix.
  Preserve `(unverified)` labels, distinguish retracted findings, and give the report path.
- **Failure:** Report the error and available log paths, redacting credentials.
  Explain the required fix without claiming the review succeeded or switching backends.

Remove only temporary worktrees created for this review.
See the [ACR user guide](../../../tools/acr/README.md) for additional options.
