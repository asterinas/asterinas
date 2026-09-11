# Aster Code Review (ACR)

ACR is a local code-review tool for Asterinas.
It reviews either the commits on your branch (`diff` mode)
or selected files in the working tree (`files` mode)
against Asterinas's [Coding Guidelines](../../book/src/to-contribute/coding-guidelines/),
and writes one Markdown report with findings and suggested fixes.

It is **local-first**: it reads a Git checkout and writes a file;
no server or pull request is needed.
It is **read-only**: review agents may read source code and Git history,
but cannot modify the repository or choose where the report is written.
It is **benchmark-driven**: a suite of known defects measures how many of them ACR catches (recall),
so improvement to design and changes to guidelines are judged by evidence.

## Quick start

You can trigger it from either inside an agent session by [ACR_SKILL](../../.agents/skills/aster-code-review/SKILL.md), 
or from a shell.

There are three backends in ACR.
1. For Openai Agents SDK, setup env firstly:

```sh
export OPENAI_API_KEY='...'
export OPENAI_BASE_URL='...'
```

If you use Pi or Claude, skip this step, make sure your chosen agent is configured locally,
and add `--config tools/acr/acr.pi.toml` (Pi) or `--config tools/acr/acr.claude.toml` (Claude) when running ACR.

2. Running ACR: review the commits your branch added on top of `origin/main`.

```sh
tools/acr/run.sh diff origin/main /tmp/acr-review.md
```

Or just review the latest commit:

```sh
tools/acr/run.sh diff HEAD^ /tmp/review.md
```

Or review files, including uncommitted edits, optionally restricted to line ranges:

```sh
tools/acr/run.sh files kernel/src/lib.rs:1-80 README.md /tmp/acr-files.md
```

When finishing, ACR prints the paths of review report.

## Reading the report

[A review report](./spec/interface.md#the-report) starts with a short summary,
followed by one section per review persona that found something:
Maintainability, Correctness, Security, Hardware, and Documentation.
Each finding gives:

- the file (or commit) and line;
- its **grounding**: a guideline short-name in backticks, such as `` `validate-at-boundaries` ``,
  or a plain-language defect class when no guideline applies;
- a severity: `critical`, `major`, `minor`, or `nit`;
- the problem, its consequence, and a **Fix.**

ACR fact-checks every finding before writing the report.
A finding whose key claim could not be confirmed is prefixed with `(unverified)`.
A finding shown to be wrong is moved to the final `Retracted by verification` section with the reason,
so you can still judge it yourself.

## Review modes

| Mode | Reviews | Uncommitted edits |
|---|---|---|
| `diff <base>` | Every commit in `merge-base(<base>, HEAD)..HEAD`, oldest first, as its message plus its diff. | Not included. Commit them first, or use `files`. |
| `files <path[:lines] ...>` | The current contents of the named files. `lines` is 1-based and inclusive, e.g. `:40`, `:40-120`, or `:40-120,200-210`. | Included. |

To review another commit, checkout first. 
`diff` always reviews commits up to the current HEAD; files reviews the current working-tree contents.

The last argument is the report path.
File targets and a relative report path are resolved from the repository root,
not from the current directory.
ACR refuses to replace an existing report unless you add `--overwrite`.
See [Interface and output](spec/interface.md) for the full syntax.

## Choosing a backend

| Backend | Use it for | Setup |
|---|---|---|
| `openai-agents` (default) | Rviews through OpenAI or compatible model APIs. | Set `OPENAI_API_KEY` and  `OPENAI_BASE_URL`. The default profile is [`acr.toml`](acr.toml). See [OpenAI Agents backend](spec/openai_agents_backend.md#how-to-use-this-backend). |
| `pi-agent` | Reviews using Pi agent. | Login to your provider in Pi first, then pass `--config tools/acr/acr.pi.toml`. See [Pi backend](spec/pi_backend.md#how-to-use-this-backend). |
| `claude-agent` | Reviews through the Claude Agent SDK, inheriting the local Claude CLI configuration. | Use [`acr.claude.toml`](acr.claude.toml). See [Claude backend](spec/claude_backend.md#how-to-use-this-backend). |

Select a backend with `--backend=<name>` or the `--config [path_to_toml]`.
If no profile and backend toml is specified, 
ACR will default to using codex with `tools/acr/acr.toml` and gpt-5.5 medium for review.
You can modify config in the profile file.
API key never go into a profile.
See [Configuration](spec/interface.md#configuration) for details.

## How it works

ACR reviews code from up to five **personas**,
each owning one part of the coding guidelines:
Maintainability, Development (shown as Correctness in the report), Security, Hardware, and Documentation.
Maintainability, Development, and Security always run.
Hardware and Documentation is path-based and deterministic.

Personas review either in separate contexts (**fan-out** by default)
or together in one shared context (**combined**).

In either mode, agents receive the code under review,
a one-line summary of each rule for their assigned personas,
and read-only tools for source code and Git history.
By default, they fetch a rule's full text only when they suspect a violation.

ACR then merges the findings, fact-checks each one,
aligns the fixes of findings that share a root cause,
writes a summary, and renders the report.
See the [execution model](spec/execution_model.md) for details.

## What's in this directory

| Path | Contents |
|---|---|
| [`run.sh`](run.sh) | Command-line entry point. |
| [`acr.toml`](acr.toml), [`acr.pi.toml`](acr.pi.toml), [`acr.claude.toml`](acr.claude.toml) | Reference profiles for the OpenAI, Pi, and Claude backends. |
| [`src/`](src/) | The `acr` Python package; [`core/`](src/core/) contains the review workflow, its steps, and shared data types. |
| [`benchmark-data/`](benchmark-data/) | Known-defect problems ([`problems.yaml`](benchmark-data/problems.yaml)) and the benchmark entry point. |
| [`tests/`](tests/) | Model-free tests. |
| [`spec/`](spec/) | The design specification. Start at [`spec/README.md`](spec/README.md). |
