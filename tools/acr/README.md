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

You can run ACR from a shell, as below,
or ask an agent to run it through the [`aster-code-review` skill](../../.agents/skills/aster-code-review/SKILL.md).

1. Choose a backend and set it up:

| Backend | Setup | Details |
|---|---|---|
| `openai-agents` (default) | `export ACR_API_KEY=...` and, for a relay, `export ACR_BASE_URL=...`. | [OpenAI Agents backend](spec/openai_agents_backend.md#how-to-use-this-backend) |
| `pi-agent` | Make sure your provider in Pi works. | [Pi backend](spec/pi_backend.md#how-to-use-this-backend) |
| `claude-agent` | Make sure the local Claude CLI works. | [Claude backend](spec/claude_backend.md#how-to-use-this-backend) |

2. Run ACR from the repository root.
   Review the commits your branch added on top of `origin/main`:

```sh
tools/acr/run.sh diff origin/main /tmp/acr-review.md
```

Review only the latest commit, with the Pi backend:

```sh
tools/acr/run.sh --config tools/acr/acr.pi.toml diff HEAD^ /tmp/acr-review.md
```

Review files, including uncommitted edits, optionally restricted to line ranges:

```sh
tools/acr/run.sh files kernel/src/lib.rs:1-80 README.md /tmp/acr-files.md
```

3. When it finishes, ACR prints the path of the report and of the run's logs.

## Review modes

| Mode | Reviews | Uncommitted edits |
|---|---|---|
| `diff <base>` | Every commit in `merge-base(<base>, HEAD)..HEAD`, oldest first, as its message plus its diff. | Not included. Commit them first, or use `files`. |
| `files <path[:lines] ...>` | The current contents of the named files. `lines` is 1-based and inclusive, e.g. `:40`, `:40-120`, or `:40-120,200-210`. | Included. |

Both modes review the current checkout; to review another commit, check it out first.
The last argument is the report path.
ACR refuses to replace an existing report unless you add `--overwrite`.
See [Interface and output](spec/interface.md) for the full syntax.

## Reading the report

A report starts with a short summary,
followed by one section per review persona that found something:
Maintainability, Correctness, Security, Hardware, and Documentation.
Each finding gives its location, the guideline rule (or defect class) it rests on,
a severity (`critical`, `major`, `minor`, or `nit`), the problem, and a **Fix.**

ACR fact-checks every finding before writing the report.
A finding whose key claim could not be confirmed is prefixed with `(unverified)`;
a finding shown to be wrong is moved to the final `Retracted by verification` section,
so you can still judge it yourself.
See [The report](spec/interface.md#the-report) for the exact format.

## Configuration

`--config` selects a profile, and the profile selects the backend.
The reference profiles are [`acr.openai.toml`](acr.openai.toml) (the default),
[`acr.pi.toml`](acr.pi.toml), and [`acr.claude.toml`](acr.claude.toml).
A profile also sets the models and reasoning effort of the two stages
— persona review and post-processing —
plus backend-specific options.
To change them, copy a reference profile, edit it, and pass it with `--config`.

For each setting, a command-line flag beats an `ACR_*` environment variable,
which beats the profile, which beats the built-in default.
Credentials never go into a profile.
See [Configuration](spec/interface.md#configuration) for every setting.

## How it works

Up to five reviewer personas review the code in parallel,
each against its own part of the guidelines and with read-only tools.
ACR then merges their findings, fact-checks each one,
aligns the fixes of findings that share a root cause, writes a summary, and renders the report.
See the [design specification](spec/README.md) for the architecture and the reasons behind it.

## What's in this directory

| Path | Contents |
|---|---|
| [`run.sh`](run.sh) | Command-line entry point. |
| [`acr.openai.toml`](acr.openai.toml), [`acr.pi.toml`](acr.pi.toml), [`acr.claude.toml`](acr.claude.toml) | Reference profiles for the OpenAI, Pi, and Claude backends. |
| [`src/`](src/) | The `acr` Python package; [`core/`](src/core/) contains the review workflow, its steps, and shared data types. |
| [`benchmark-data/`](benchmark-data/) | Known-defect problems ([`problems.yaml`](benchmark-data/problems.yaml)) and the benchmark entry point. |
| [`tests/`](tests/) | Model-free tests of the whole system; run `make -C tools/acr/tests`. Tests that need Node or Pi are skipped when those are absent. |
| [`spec/`](spec/) | The design specification. Start at [`spec/README.md`](spec/README.md). |
