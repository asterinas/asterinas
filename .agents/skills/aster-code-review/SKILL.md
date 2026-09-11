---
name: aster-code-review
description: Guide the user through configuring ACR credentials, choosing diff or files mode, and reviewing Asterinas commits or files. Run ACR and summarize its Markdown report.
---

# Aster Code Review

Follow the steps in order. Wait for user's answers before moving on.
Do not start a review until credentials, mode, and targets are ready.

## 1. Check ACR credentials

Check `~/.bashrc` environment variables for non-empty `OPENAI_API_KEY` and `OPENAI_BASE_URL` values.
Report only whether each variable is set; never print its value.
Do not read these values from this chat agent's provider configuration.

If either is missing, ask the user to provide the missing value:

> ACR needs an API key and an API endpoint URL. Please provide the missing values:
> - `OPENAI_API_KEY`: your API key for ACR.
> - `OPENAI_BASE_URL`: your provider's API base URL, for example `https://api.openai.com/v1`.
>
> These values will be passed only to ACR for this review. They will not be used to configure this agent or other tools.

Help user set the values for ACR process only.
Use a secure secret-input facility when available. Do not echo the key,
write credentials into files, or change shell startup files or global settings.
If secure input is unavailable, guide user to configure the variables in the environment
that launches this skill, then check again.
Do not include the API key directly in commands that may be saved in logs or shell history.

## 2. Ask the user to choose a mode

Show both choices and wait for an answer:

> Which review mode would you like?
> 1. **diff** — review committed changes, including commit messages. Uncommitted edits are not included.
> 2. **files** — review selected files as they are now, including uncommitted edits.

## 3. Ask what to review

Ask only the question for the selected mode, then wait for the answer.

**diff:**

> Which commit would you like to review? Enter a commit ID, or `HEAD` for the latest commit on your current branch.
> To review several commits, provide the base and the last commit, for example `origin/main..HEAD`.

- ACR's `diff <base>` reviews commits up to the checkout's `HEAD`.
- For one ordinary commit, make it `HEAD` and use its parent as the base: `diff HEAD^`.
- For a series, make the last commit `HEAD` and pass only the base to ACR, not `base..head`.
- If another commit must be checked out, use a temporary detached worktree and invoke the original ACR tool by its absolute path.
  Do not switch branches or alter user's working tree. Keep review report outside the temporary worktree.
- If the commit is a merge commit or the repository's first commit, explain why `diff HEAD^` cannot review it alone and ask user which files or commit range to review instead.

**files:**

> Which files would you like to review? Enter paths relative to Asterinas repository root.
> For example, `kernel/src/lib.rs` reviews the whole file; `kernel/src/lib.rs:10-80` reviews lines 10 through 80.
> You can provide several files. To review uncommitted changes, say so and I will list the changed files for you to confirm.

If asked to find changed files, use `git status --short` and confirm the file list with user.
Exclude unrelated temporary logs and generated files. Quote each path when invoking ACR.

## 4. Run ACR and read the report

Run from the root of the checkout being reviewed.
Use user's report path, or create a new one:

```sh
report="$(mktemp -d /tmp/acr-review.XXXXXX)/review.md"
```

Run the command that matches the user's chosen mode and targets:

```sh
tools/acr/run.sh --backend=openai-agents diff <base> "$report"
# Or:
tools/acr/run.sh --backend=openai-agents files <path[:lines] ...> "$report"
```

Pass the credentials only to this ACR invocation. Do not use `fake` backend.
Use `--overwrite` only if user permits replacing an existing report.
See [the user guide](../../../tools/acr/README.md) for more options.

Wait for completion and check the exit status.
- **Success:** read the report and summarize findings with file/line, severity, and suggested fixes.
  Keep `(unverified)` labels, distinguish retracted findings, and give the report path.
- **Failure:** report the error and log paths. Do not claim the review succeeded.

Do not modify reviewed code or post to GitHub.
Remove only temporary worktrees you created, once the review has finished.
