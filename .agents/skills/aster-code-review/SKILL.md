---
name: aster-code-review
description: Review code against Asterinas's persona-keyed coding guidelines and write a Markdown review file. Use when asked to review a Git change (diff mode) or a set of target files (files mode) for defects, or inside a write-test-review loop.
---

# aster-code-review

Review code against Asterinas's coding guidelines
— the persona-keyed pages under `book/src/to-contribute/coding-guidelines/`
— and write one Markdown review file.
The review is **recall-first**: miss as few real defects as possible.
It makes both objective calls (undeniable bugs) and subjective calls (guideline violations, each grounded in a cited rule).

There are two review modes, both **anchored at the current checkout (HEAD)**:

- **`diff <base>`** — a Git change:
  the **commit series** the branch adds over its merge-base with `<base>`
  — each commit's message *and* its diff,
  so the review covers per-commit intent and commit hygiene, not just the net change.
  (Commit first; uncommitted edits are not reviewed in diff mode.)
- **`files <path[:lines] ...>`**
  — target code: the working-tree contents of the named files,
  optionally narrowed to line ranges.

To review at a specific commit, check it out first (it becomes HEAD).

This skill is agent-agnostic (Claude Code and Codex).
Only step 3's spawn primitive differs between agents;
everything else is identical.

## Interface

However the skill is triggered,
it receives ONE raw argument string that begins with a mode word:

```
diff   <base>              <output> [--overwrite] [--per-persona-context=auto|yes|no]
files  <path[:lines] ...>  <output> [--overwrite] [--per-persona-context=auto|yes|no]
```

- `<base>` (diff) — **required**,
  any ref or SHA;
  reviews the commit series `merge-base(<base>, HEAD)..HEAD` (each commit's message + diff).
- `<path[:lines] ...>` (files) — **required**, one or more targets in the working tree.
  A target is a path, optionally `path:N-M` or `path:N-M,K-L` (1-based, inclusive);
  repeat a path to add ranges.
  Wrap a path in double quotes to allow spaces or colons (`"a: b.rs":10-20`).
- `<output>` — **required, last positional** (`cp src... dest` style);
  refuse to overwrite unless `--overwrite`.
- `--overwrite` — replace the output file if it already exists.
- `--per-persona-context` — `yes`: fan out one isolated agent per persona (best recall).
  `no`: one combined agent reviews all personas (cheaper, lower recall).
  `auto` (default) currently resolves to `yes`;
  it will become a benchmark-driven heuristic.
  Controls step 3 only.

**Do not pre-split the argument string yourself.**
Pass it verbatim, as a single quoted argument,
to `resolve_target.sh` (step 1)
— the script self-tokenizes (whitespace-split except inside double quotes),
which keeps the parse deterministic and identical across agents.

## Pipeline

Run these steps in order.
Steps 1 and 5 are deterministic scripts;
steps 6–8 use the model.

The helper scripts live in the skill's own `scripts/` directory,
at `.agents/skills/aster-code-review/scripts/` inside the repository under review.
**Invoke every script by its absolute path**
— the working directory is the code under review (the repo root), *not* the skill directory,
so a bare `scripts/…` is not found (notably on Codex, which runs commands from the repo root; do not `cd` into the skill dir either — `resolve_target.sh` needs the repo as its cwd).
Shells do not persist between commands,
so **set `SKILL` at the start of each command that uses it** (or inline the `$(git rev-parse …)`):

```sh
SKILL="$(git rev-parse --show-toplevel)/.agents/skills/aster-code-review"; "$SKILL/scripts/resolve_target.sh" '<raw args>'
```

The steps below write `$SKILL/scripts/…` as shorthand for that absolute path.

1. **Resolve the target.**
   `"$SKILL/scripts/resolve_target.sh" '<raw args>'` prints the canonical review input
   — the commit series (each commit's message + diff) in `diff` mode,
   annotated file excerpts in `files` mode;
   save it to a temp file.
   Run it again with `--meta` first (`"$SKILL/scripts/resolve_target.sh" --meta '<raw args>'`) to get
   `mode=`, `base=`/`files=`, `head=`, `branch=`, `output=`, `overwrite=`, `per_persona_context=`;
   add `date=` (today) and an optional `title=` to make the meta file.
   Pass the raw argument string as a single quoted argument
   so the script's tokenizer sees it intact.
2. **Activate personas.**
   Pick which of the five personas run,
   from the reviewed paths — changed paths in `diff` mode,
   named paths in `files` mode (see *Activation*).
3. **Fan out.**
   Spawn the persona passes (see *Spawning*).
   With `per_persona_context` = `yes` or `auto` (the default),
   run **one isolated PASS per activated persona** — best recall.
   With `no`, run **one combined PASS** over all activated personas in a single context
   — cheaper, lower recall.
   Build each pass's prompt deterministically with `"$SKILL/scripts/build_pass_prompt.sh" <input-file> <persona>...`
   — one persona for fan-out, all activated personas for combined
   — and spawn the sub-agent with **that exact text**.
   Each pass returns a JSON array of comments;
   file each under its persona's `<fragdir>/<persona>.json` (the comment's `persona` field says which),
   so step 5 is unchanged.
4. **Collect** the per-persona JSON fragments.
5. **Assemble.**
   `"$SKILL/scripts/assemble_review.sh" [--overwrite] <meta> <fragdir> <output>` performs the deterministic merge (group by persona in fixed order, sort by file→line, drop exact duplicates *within a persona*, write frontmatter, leave a `<!-- SUMMARY -->` placeholder).
   Pass `--overwrite` only if the user gave it;
   otherwise the script refuses to clobber an existing `<output>`.
6. **Verify** (see *Verification*).
7. **Consolidate** (see *Consolidation*).
8. **Summary.**
   Replace `<!-- SUMMARY -->` with a constructive summary over the final comments:
   what the code does well, the top issues ranked by severity,
   any structural recommendations.

## Activation

A persona runs unless the reviewed paths *provably* contain nothing in its remit (reviewed paths = changed paths in `diff` mode, named paths in `files` mode):

- **maintainability, development, security** — any reviewed code.
- **hardware** — a reviewed path is assembly (`*.S`, `*.asm`),
  an architecture directory (`.../arch/...`), or contains `asm!` / `global_asm!`.
- **documentation** — a reviewed path is `book/`, any `*.md`,
  syscall-coverage files (`*.scml`), or a user-facing API surface (a syscall or kernel parameter).

Activation is path-based and deterministic;
do not use a model to triage (a wrongly-skipped persona is a silent recall hole).

## Pass contract

The shared reviewer contract
— the review methodology and the JSON comment schema every pass must follow
— lives in [`scripts/pass_contract.md`](scripts/pass_contract.md),
inlined as the **stable head** of every pass prompt by `build_pass_prompt.sh` (step 3).
Do not paraphrase it into the prompt yourself:
retyping it breaks the byte-identical prefix the prompt cache relies on (see [`execution_model.md`](spec/execution_model.md)).
A pass reads only the persona block(s) it is given (selective exposure),
reviews the **REVIEW INPUT** at the foot of the prompt,
and returns a JSON array of comments per that schema.
Each persona block contains the persona's complete short-name/gist catalog,
not every rule body.
On a concrete suspected violation,
the pass batches the relevant short-names through
`python3 .agents/skills/aster-code-review/scripts/guideline_query.py show --expect-digest <catalog-digest> <persona> <short-name>...`
and reads those exact authored rule chunks before citing them.
The query tool selects the same current or benchmark-snapshotted guideline corpus
that built the catalog.

`ACR_GUIDELINE_DISCLOSURE=full` is an internal benchmark/rollback switch
that restores eager subpage inlining;
the default is `progressive` and the switch is not part of the user interface.

## Spawning a pass

Build every pass prompt with `"$SKILL/scripts/build_pass_prompt.sh" <input-file> <persona>...`
and spawn the sub-agent with its **exact** output
— only a script keeps the prompt's stable head (contract + persona/catalog) byte-identical across passes and reviews,
which is what lets the prompt cache reuse it (see [`execution_model.md`](spec/execution_model.md)).

**Default (`per_persona_context` = `yes`/`auto`)**
— one pass per activated persona,
each in a CLEAN context with only its own persona block (selective exposure):

- **Claude Code** — spawn a Task sub-agent per persona.
- **Codex** — run `codex exec "<build_pass_prompt.sh output>"` per persona
  (pass the built PROMPT TEXT as the argument).

**Never spawn a pass by re-running `aster_code_review.sh` / `run_agent.sh`,
nor by re-issuing the skill's own arguments** (e.g. `codex exec … diff <base> <out>`).
A pass is a *reviewer* invocation whose input is the `build_pass_prompt.sh` text
— not another run of this skill.
It is already inside the active workflow and must not load this `SKILL.md` again.
Re-entering the launcher spawns another orchestrator that spawns another … , an infinite fork bomb;
the launcher now refuses it (`ACR_AGENT_RUNNING`).

Passes are independent;
order does not matter.
Collect each pass's JSON.

**Combined (`per_persona_context` = `no`)**
— build ONE prompt with all activated personas (`"$SKILL/scripts/build_pass_prompt.sh" <input> <p1> <p2> ...`)
and run a single pass;
it reviews through every lens at once and tags each comment with its `persona`.
The input and spawn overhead are paid once,
so it is cheaper
— but a larger shared context dilutes focus,
so it finds fewer defects.
The default stays fan-out:
recall comes first,
and `no` is an explicit, opt-in concession.

## Verification (step 6)

For each comment, isolate the key premise it rests on
— especially an external-system fact (Linux/POSIX behaviour, the System V ABI, Rust semantics).
Try to **refute** it:
re-read the cited code and consult an authoritative source.
Assign a verdict:

- **confirmed** — keep the comment unchanged.
- **uncertain** — keep it, but prefix `problem` with `(unverified) `.
- **refuted** — remove the comment,
  and append it to a `## Retracted by verification` list at the foot of the file with a one-line reason.

Remove **only** on confident refutation;
an unsure check is `uncertain`, not `refuted`.
This is the only step that may remove a comment, and only false positives.

## Consolidation (step 7)

Find clusters of comments that share one root cause or one fix
(e.g. several manual lock/unlock pairs that all want RAII).
For each cluster, write a single unified fix
and repoint each member's `**Fix.**` at it ("Shared with the other `raii` comments: …").
**Never remove a comment**
— every symptom stays at its own location.

## Output format

The assembled file is:
frontmatter (`date`, `mode`, `base` or `files`, `head`, `branch`, optional `title`),
a `# Summary`,
then a `## <section>` per persona in the order **Maintainability, Correctness, Security, Hardware, Documentation** (Correctness is the Development persona's section).
Each comment is a `### \`file\` line N` subsection:
a quoted ` ```diff ` snippet,
then a prose line `` `<grounding>` (<severity>): <problem> ``,
then a `**Fix.**` paragraph.
Do not restructure what the script emits;
only fill the summary and apply the verify/consolidate edits.
