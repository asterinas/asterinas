# Interface and Output

## Entry point

Run ACR through [`run.sh`](../run.sh) from the repository to review. The script
locates the Python package and a trusted guideline corpus, then invokes
[`run_review.py`](../run_review.py).

```text
run.sh [--backend=openai-agents|pi-agent|fake] [--config PATH] \
  diff  <base>             <output> [--overwrite]

run.sh [--backend=openai-agents|pi-agent|fake] [--config PATH] \
  files <path[:lines] ...> <output> [--overwrite]
```

`--backend` and `--config` are process options. The remaining tokens form the
raw review interface and are resolved deterministically by
[`scripts/resolve_target.sh`](../scripts/resolve_target.sh). The output is
mandatory and is always the last positional argument.

Examples:

```sh
./run.sh diff origin/main /tmp/review.md --backend=fake --overwrite
./run.sh files kernel/src/fs/file.rs:40-120 review.md --config ./acr.toml
```

`ACR_PYTHON` may select the Python interpreter. OpenAI-backed runs require the
credential environment variable named by the selected configuration. Pi-backed
runs use Pi's own saved authentication and model registry.

## HEAD is the head

Both modes are anchored at the current checkout. There is no separate head
argument.

### `diff <base>`

Diff mode reviews the commits in
`merge-base(<base>, HEAD)..HEAD`, oldest first. Each commit message and patch is
included so the reviewer can judge intent and commit hygiene as well as the net
code change. Uncommitted changes are not included; commit them or use file
mode. An empty commit range fails instead of producing an empty review input.

### `files <path[:lines] ...>`

File mode reviews working-tree contents. A target may be a whole file, a single
1-based line, or inclusive ranges:

```text
kernel/src/sched/fair.rs
kernel/src/sched/fair.rs:120-180
kernel/src/sched/fair.rs:120-180,240-260
```

Repeated paths are combined and overlapping or adjacent ranges are merged.
The selected lines are the review scope; read-only tools may inspect surrounding
repository context.

Double-quote paths containing spaces, colons, or an ambiguous range-looking
suffix. The resolver owns this quoting grammar; it does not evaluate shell
syntax. A trailing `:RANGES` is removed only when the suffix fully matches the
line-range grammar.

## Review-interface flags

| Flag | Meaning |
|---|---|
| `--overwrite` | Permit replacement of an existing report. Without it, publication fails closed. |
| `--per-persona-context=auto\|yes\|no` | Compatibility token accepted by target resolution and recorded in metadata. Runtime fan-out is selected by configuration as described below. |

The runtime's effective persona-context policy is the
`agent.per_persona_context` configuration value (or
`ACR_PER_PERSONA_CONTEXT`). `auto` currently resolves to `yes`; `no` runs one
combined review invocation. The benchmark also exposes an explicit
`--per-persona-context` process option.

## Configuration

[`config.py`](../config.py) validates the runtime configuration. Value
precedence is:

1. process-level overrides such as `--backend`;
2. `ACR_*` environment variables;
3. the selected TOML document;
4. built-in defaults.

An explicit `--config` path must exist and is never merged with another file.
Without one, the loader uses `acr.toml` in the current directory when present,
then the package's [`acr.toml`](../acr.toml). OpenAI-provider credentials are
never read from ACR TOML; `provider.api_key_env` names the environment variable
to read. Pi credentials remain in Pi-owned configuration.

Important groups are:

- model, reasoning effort, fan-out, concurrency, timeouts, and retries under
  `[agent]`;
- per-persona and post-processing agent allowlists under `[tools]`;
- provider adapter, endpoint, credential-variable name, and compatibility
  filters under `[provider]` for `openai-agents`;
- Node command, Pi agent directory, bridge lifecycle, native-session policy,
  and explicitly trusted extensions under `[pi]` for `pi-agent`;
- local observability payload and size controls under `[tracing]`.

`adapter = "openai"` uses the SDK's native OpenAI-compatible Responses or Chat
Completions model. `adapter = "any-llm"` uses the optional Any-LLM adapter.
Adding `web.search` to an agent's `[tools]` allowlist requires
`wire_api = "responses"` and endpoint support.

`backend = "pi-agent"` selects [`acr.pi.example.toml`](../acr.pi.example.toml)
as the reference profile. Its model is a Pi `provider/model-id` reference. Pi
reads relay credentials, Claude subscription state, `auth.json`, and
`models.json` from `pi.agent_dir`; ACR does not copy those credentials into
TOML. `ACR_PI_NODE_COMMAND` and `ACR_PI_AGENT_DIR` override the corresponding
Pi settings for local execution. See [Pi backend](pi_backend.md).
See the configuration discussion in the [runtime README](../README.md).

## Markdown report

The final report contains YAML frontmatter, a summary, and one section per
persona that produced findings:

````markdown
---
date: 2026-09-06
mode: files
files: kernel/src/example.rs:10-30
head: 1a2b3c4-dirty
branch: feature
---

# Summary

<constructive synthesis>

## Correctness

### `kernel/src/example.rs` line 18

> ```diff
> +    let value = entries[index];
> ```

Bounds check missing (major): A user-controlled index can panic the kernel.

**Fix.** Validate the index before indexing and return the documented error.
````

Diff-mode frontmatter records the merge-base and HEAD. File mode records the
selected files and appends `-dirty` to the head token when tracked working-tree
or index changes exist.

The fixed section order is Maintainability, Correctness (the Development
persona), Security, Hardware, and Documentation. Empty persona sections are
omitted.

Each comment is represented internally by the strict `ReviewComment` contract
in [`core/contracts.py`](../core/contracts.py):

- repository-relative file or commit locus, and an optional source line;
- owning persona;
- guideline short-name or plain-language defect class;
- `critical`, `major`, `minor`, or `nit` severity;
- concrete problem, consequence, and proposed fix;
- an optional minimal diff or source excerpt.

Verification may mark a finding `(unverified)` or move a confidently refuted
finding to `## Retracted by verification`. Consolidation may rewrite fixes but
does not remove findings. See
[Execution model](execution_model.md#verification-consolidation-and-summary).

## Optional GitHub publication

Report generation has no GitHub side effect. An explicit caller may pass a
completed report to [`scripts/post_reviews_to_github.sh`](../scripts/post_reviews_to_github.sh)
or [`sinks/github.py`](../sinks/github.py). The publisher validates the expected
PR head when GitHub returns it, places attachable findings inline, keeps
off-diff findings in the review body, and leaves the review pending unless
`--finalize` is requested.
