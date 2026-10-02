# Benchmark and Evaluation

The benchmark measures whether ACR finds known defects in Git changes and
existing files.
It is a separate control plane built around detached worktrees, optional
structured grading, and strict expected-defect recall.

The benchmark is an optimization signal, not a leaderboard.
With a small corpus, every miss should lead back to a concrete weakness in a
guideline, prompt, verification decision, or orchestration step.
Runs are comparable only when their inputs and configuration are pinned;
success with the fake backend establishes machinery health, not review quality.

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
- problem_id: <slug>              # REQUIRED, unique (numeric part too). number + kebab slug, e.g. 0004-semop-dead-timer-retain
  commit: <rev>                   # REQUIRED. the snapshot to check out (detached HEAD):
                                  #   diff mode -> a full 40-char SHA (fetched by SHA);
                                  #   files mode -> any local commit-ish (e.g. f4e29d67c^).
  remote: <fetch URL>             # OPTIONAL. where to fetch `commit`; defaults to https://github.com/asterinas/asterinas.
  source: >                       # REQUIRED. freeform provenance + why the problem is leak-free
    ...
  review_mode:                    # REQUIRED. EXACTLY ONE of `diff` / `files`.
    diff:                         #   diff mode: review `base..HEAD` (each commit's message + diff).
      base: <rev>                 #     REQUIRED ref relative to the checkout; HEAD^ for a single introducing commit.
    files:                        #   files mode: targets reviewed at `commit` (whole-file is the norm)
      - <path[:lines]>
  defects:                        # REQUIRED, one or more — the ground truth
    - target:                     #   REQUIRED
        kind: <kind>              #     REQUIRED: file | commit_message | whole_change
        path: <path>              #     REQUIRED iff kind: file
        lines: "<a-b>"            #     OPTIONAL, only when kind: file
      persona: <persona>          #   REQUIRED: maintainability|development|security|hardware|documentation
      grounding: <name>           #   REQUIRED: a guideline short-name, or a short plain-language defect description
      severity: <level>           #   REQUIRED: critical | major | minor | nit  (informative only)
      desc: >                     #   REQUIRED. what is wrong — context for the grader and humans
        ...
      fix: >                      #   REQUIRED unless is_negative — the concrete remedy
        ...
      expectation: >              #   REQUIRED. the criterion a review comment is matched against
        ...
      is_negative: false          #   OPTIONAL, default false. true = false-positive trap (omit fix)
```

Diff problems require a full 40-character commit SHA because the controller may
fetch the exact object from `remote`; an omitted remote uses `benchmark.remote`
from the selected ACR config. File problems may use a locally resolvable commit-ish.
`review_mode` contains exactly one of `diff: {base: ...}` and a non-empty
`files: [...]` list.

Each defect has a target kind (`file`, `commit_message`, or `whole_change`), an
owning persona, grounding, severity, description, expectation, and normally a
fix.
`desc` explains what is wrong for the grader and human readers.
`expectation` is the strict `MATCH IF` criterion used to decide whether a review
finding caught the defect.
It should be concise and checkable, not merely a paraphrase of `desc`.
`severity` is informative metadata and does not affect the recall gate.

The schema reserves `is_negative: true` entries for precision traps.
They omit the fix and are excluded from the current strict-recall denominator.
Precision scoring for them is future work.
`runner.load_problems()` enforces these invariants before creating any worktree
or starting an agent.

The corpus itself is the current inventory; this specification deliberately
does not duplicate its problem list.

## Honest problem sourcing

A problem must not reveal its answer in the canonical review input.
Good sources are:

- the original introducing commit for a defect later fixed in history, which is
  the preferred diff-mode source;
- a realistic synthetic commit that does not name or otherwise hint at the
  planted defect;
- the whole affected file at the parent of a fixing commit, which is the
  preferred file-mode source.

File-mode problems should normally review whole files rather than a suspiciously
narrow range. Reverse-of-fix patches are avoided because deleted guards,
assertions, tests, and comments often point directly at the answer.

Full commit identities make Git the single source of truth and keep fixtures
reproducible without maintaining a patch copy that can drift from its commit.
Diff problems therefore use full 40-character SHAs.
If an object is absent locally, the worktree manager fetches the named commit
from the configured remote.
This retains a dependency on that remote continuing to serve the object.

## Answer-key separation and isolation limits

The controller keeps expected defects out of the review request and introduces
them to the grader only after review generation.
For each problem, it:

1. creates an opaque detached worktree at the problem commit;
2. removes historical `.agents` and `.claude` resources, then overlays the
   current ACR package while excluding the entire benchmark directory;
3. binds guideline access to the controller's trusted repository, not the
   historical worktree;
4. calls the ordinary ACR orchestrator with only the mode, targets, and output;
5. removes the worktree;
6. only then, when `--grade` is set, gives expected defects and the produced
   report to a separate grader invocation.

These steps keep the answer key out of the canonical review input, overlaid
working tree, prompts, and controller-supplied reviewer artifacts.
The overlay exclusion is essential: copying `problems.yaml` into the target
would make the answer key directly available through source tools.
Historical prompts, personas, and guidelines are likewise not selected from
the target checkout.

This is logical control-plane separation, not complete Git-object isolation.
A detached worktree shares its object database and refs with the controller
checkout.
When reviewer tools permit history-wide queries or arbitrary object expressions,
such as `git.log --all` or `git.show <revision>:<path>`, a reviewer may be able
to inspect answer-bearing objects that are absent from the overlaid tree.
Excluding `benchmark/` from the overlay does not close that path.

A formal leak-resistant evaluation must instead use an independent temporary
repository or object store containing only an audited revision closure, or
restrict the Git broker to an explicit set of review revisions and paths.
Ground truth must remain outside that boundary and be supplied only to the
post-review grader.

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

Matching is semantic and model-graded, not string matching.
A finding may catch a defect using different wording, while a nearby but
materially different observation must not receive credit.
Recall also exercises post-processing: if verification incorrectly retracts a
valid persona finding, the final report misses the expectation and the recall
score records that failure.
Negative problems are reserved as a future precision guardrail against gaining
recall by reporting indiscriminately.
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
