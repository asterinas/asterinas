# Benchmark and Evaluation

The benchmark measures whether ACR finds known defects. It is a separate
control plane built around detached worktrees, optional structured grading,
and strict expected-defect recall.

Its implementation lives in [`benchmark/`](../benchmark/):

| File | Responsibility |
|---|---|
| [`problems.yaml`](../benchmark/problems.yaml) | Versioned review problems and grader-only ground truth. |
| [`runner.py`](../benchmark/runner.py) | Selection, worktree setup, review runs, optional grading, and aggregate output. |
| [`worktree.py`](../benchmark/worktree.py) | Exact-commit fetch and detached-worktree lifecycle. |
| [`grader.py`](../benchmark/grader.py) | Structured grader invocation and strict recall calculation. |
| [`run.sh`](../benchmark/run.sh) | Location-independent command-line entry point. |

## Running the benchmark

Install the optional `acr[benchmark]` dependency for YAML loading, then run
from an Asterinas checkout:

```sh
./benchmark/run.sh --problem 0002 --backend fake
./benchmark/run.sh --problem 0002 --backend openai-agents \
  --config ./acr.toml --grade
ACR_PI_NODE_COMMAND=/absolute/path/to/node \
  ./benchmark/run.sh --problem 0002 --backend pi-agent \
  --config ./acr.pi.example.toml --grade
```

Useful options include:

- repeatable `--problem <id-or-prefix>` selection;
- `--repo <checkout>` for the repository that supplies Git objects;
- `--work <directory>` for retained output;
- `--backend`, `--config`, `--timeout`, `--retries`, and `--max-turns`;
- `--per-persona-context=auto|yes|no`;
- `--grade` to start a separate grader after each completed review.

Without `--work`, OpenAI and fake runs are stored under `$ACR_BENCHMARK_ROOT`
or `/tmp/aster-code-review/benchmarks/`. Their directory names contain a GMT+8
local timestamp, selected numeric problem IDs, and a unique suffix.

Pi runs use the separate `$ACR_PI_BENCHMARK_ROOT` or
`/tmp/aster-code-review/benchmarks-pi/` root. A single-problem directory is
named `run-<GMT+8 timestamp>-<full problem ID>-<random hex>`; the random string
is last. Review logs, response artifacts, tool transcripts, token usage, Pi
session directories, the report, and optional grader logs remain below that
directory. Detached worktrees themselves are temporary and removed after each
review.

## Problem schema

The corpus is a YAML sequence. Each problem has one checkout commit, exactly
one review mode, provenance, and one or more expected defects:

```yaml
- problem_id: 0002-fair-weight-race
  commit: <commit-ish>
  remote: <optional-fetch-url>
  source: >
    Provenance and leak analysis.
  review_mode:
    diff:
      base: HEAD^
    # Or: files: [path/to/file.rs]
  defects:
    - target:
        kind: file
        path: kernel/src/example.rs
        lines: "10-20"
      persona: development
      grounding: careful-atomics
      severity: major
      desc: >
        Human-readable defect description.
      fix: >
        Expected remedy.
      expectation: >
        Crisp semantic criterion used by the grader.
      is_negative: false
```

Diff problems require a full 40-character commit SHA because the controller may
fetch the exact object from `remote`; an omitted remote uses `benchmark.remote`
from the selected ACR config. File problems may use a locally resolvable commit-ish.
`review_mode` contains exactly one of `diff: {base: ...}` and a non-empty
`files: [...]` list.

Each defect has a target kind (`file`, `commit_message`, or `whole_change`), an
owning persona, grounding, severity, description, expectation, and normally a
fix. The schema reserves `is_negative: true` entries for precision traps; they
omit the fix and are excluded from the current strict-recall denominator.
Precision scoring for them is future work. `runner.load_problems()` enforces
these invariants before creating any worktree or starting an agent.

The corpus itself is the current inventory; this specification deliberately
does not duplicate its problem list.

## Honest problem sourcing

A problem must not reveal its answer in the review input. Good sources are:

- the original introducing commit for a defect later fixed in history;
- a realistic synthetic commit that does not name the planted defect;
- the whole affected file at the parent of a fixing commit.

File-mode problems should normally review whole files rather than a suspiciously
narrow range. Reverse-of-fix patches are avoided because deleted guards,
assertions, tests, and comments often point directly at the answer.

Full commit identities keep fixtures reproducible without maintaining a second
patch copy. If an object is absent locally, the worktree manager fetches only
the named commit from the configured remote.

## Answer-key and resource isolation

The controller loads expected defects, but the reviewer never receives them.
For each problem it:

1. creates an opaque detached worktree at the problem commit;
2. removes historical `.agents` and `.claude` resources, then overlays the
   current ACR package while excluding the entire benchmark directory;
3. binds guideline access to the controller's trusted repository, not the
   historical worktree;
4. calls the ordinary ACR orchestrator with only the mode, targets, and output;
5. removes the worktree;
6. only then, when `--grade` is set, gives expected defects and the produced
   report to a separate grader invocation.

The overlay exclusion is essential: copying `problems.yaml` into the target
would make the answer key available through source tools. Historical prompts,
personas, and guidelines are likewise not selected from the target checkout.

## Structured grading

Expected non-negative defects are rendered as a numbered list with one
`MATCH IF` criterion each. The grader returns exactly one structured result per
number:

```json
{
  "results": [
    {"defect": 1, "status": "caught", "reason": "..."}
  ]
}
```

[`benchmark/grader.py`](../benchmark/grader.py) rejects duplicate, missing,
unknown, or non-contiguous defect IDs. Status is one of:

- `caught`: all material parts of the expectation are satisfied;
- `partial`: the same concern appears but a material part is absent or vague;
- `miss`: no finding satisfies the expectation.

Strict recall is:

```text
caught / expected non-negative defects
```

Only `caught` contributes to the numerator. Partial and missed defects are
printed individually with their expectation and grader reason. Multi-problem
runs also print aggregate strict recall. Evaluation failures fail closed rather
than inventing a score.

The grader is created through the same backend factory as the reviewer. A Pi
benchmark therefore uses another fresh Pi child with the grader system prompt,
grader-only user input, no review tools, and the strict `GraderEnvelope`
schema. Ground truth is still introduced only after report generation.

## Model-free and model-backed checks

The fake backend exercises worktree setup, package overlay, orchestration,
artifact production, and cleanup without network or model cost. It is a
machinery check, not evidence of review quality.

Model-backed runs evaluate review behavior. `--grade` adds another model call
per selected problem and should be treated as an explicit cost checkpoint. A
comparison should pin the problem selection, ACR revision, guideline corpus,
model, configuration, and grading method.

Pi terminal events report input/output/cache tokens, turns, tool calls, model
duration, and total adapter duration when the provider supplies them. The
per-agent and run-level `usage.json` files aggregate only reported values and
retain coverage metadata.
