# Benchmark and Evaluation

A review tool is only as trustworthy as the evidence that it finds defects.
The benchmark is a suite of **problems**, each a piece of real or realistic code with known defects.
It runs ACR on each problem and asks a separate **grader** agent which of the known defects the report caught.
The headline metric is **recall**: the fraction of known defects caught.

The benchmark is a tool for improving ACR, not a leaderboard.
The suite is small, so every miss should be traced to a concrete cause:
a missing or vague guideline, a prompt, a wrong verification verdict, or the pipeline.

## Running it

Run from the root of Asterinas:

```sh
# Check the pipeline without a model (reports are empty, no grading).
tools/acr/benchmark-data/run.sh --problem 0002 --backend fake

# Review with the default backend, then grade.
tools/acr/benchmark-data/run.sh --problem 0002 --grade

# Same with Pi.
tools/acr/benchmark-data/run.sh --problem 0002 --config tools/acr/acr.pi.toml --grade

# Same with Claude, keeping reports and logs in a chosen directory.
tools/acr/benchmark-data/run.sh --problem 0002 --config tools/acr/acr.claude.toml --grade --work /tmp/acr-benchmark
```

| Option | Meaning |
|---|---|
| `--problem <id>` | Select problems by full ID or prefix, e.g. `0002`. Repeatable. Default: all problems. |
| `--grade` | Grade each report and print recall using a separate grader run per problem. |
| `--backend`, `--config` | As for `run.sh`; see [Interface and output](interface.md#configuration). |
| `--per-persona-context=auto\|yes\|no` | Override the profile's persona-context setting. |
| `--timeout <seconds>` | Set a wall-clock limit per agent attempt. Reference profiles leave this unset. |
| `--retries <count>` | Override retries of a complete agent run, in addition to its initial attempt. |
| `--work <dir>` | Where to keep reports and logs. Default: a new directory under `/tmp/aster-code-review/benchmarks/` (`benchmarks-pi/` for Pi), or `$ACR_BENCHMARK_ROOT` / `$ACR_PI_BENCHMARK_ROOT`. |

For each problem, the work directory keeps the report (`<problem>-<timestamp>.review.md`)
and the run records under `logs/` (see [Execution model](execution_model.md#run-records)).
With `--grade`, the runner prints for each problem:

```text
[acr] Recall 0004-semop-dead-timer-retain: 50.00% (1/2 caught; partial=1, miss=0)
[acr] Defects not caught 0004-semop-dead-timer-retain:
[acr]   defect 2 [partial] kernel/src/ipc/semaphore/...: A reviewer should ... | grader: ...
```

and an overall recall when several problems are selected.
A run with the `fake` backend proves only that the pipeline works, not that reviews are good.

## Problem file

All problems live in one file, [`benchmark-data/problems.yaml`](../benchmark-data/problems.yaml),
so they can be read and queried together; for example:

```sh
yq '.[] | select(.defects[].persona == "security") | .problem_id' tools/acr/benchmark-data/problems.yaml
```

Each problem names one commit to check out, exactly one review mode, its provenance, and its defects:

```yaml
- problem_id: 0004-semop-dead-timer-retain   # unique; the number must be unique too
  commit: <full 40-character SHA>            # diff mode: a full SHA; files mode: any revision
  remote: <Git URL>                          # optional; where to fetch `commit`;defaults to https://github.com/asterinas/asterinas.
  source: >                                  # where the problem comes from, and why it does not leak its answer
    ...
  review_mode:                               # exactly one of:
    diff:
      base: HEAD^                            #   review base..commit
    # files:
    #   - kernel/src/foo.rs                  #   review these files at `commit`
  defects:                                   # one or more
    - target:
        kind: file                           # file | commit_message | whole_change
        path: kernel/src/foo.rs              # only for kind: file
        lines: "120-140"                     # optional, only for kind: file
      persona: development                   # the persona that owns this defect
      grounding: raii                        # guideline short-name, or a short defect description
      severity: major                        # critical | major | minor | nit (informational)
      desc: >                                # what is wrong, for the grader and for humans
        ...
      fix: >                                 # the remedy; omit when is_negative
        ...
      expectation: >                         # the criterion a finding must meet to count
        A reviewer should ...
      is_negative: false                     # optional; true marks a false-positive trap
```

**`desc` and `expectation` are split on purpose:** `desc` describes the bug (context the grader and a human reader need),
while `expectation` is the *explicit criterion the grader keys on*
— "a comment matches iff it meets the `expectation`"
— phrased "A reviewer should …".
`severity` is informational and does not affect scoring;
it uses the same `critical|major|minor|nit` vocabulary the review comments render.

If `remote` is omitted, the runner uses `benchmark.remote` from the profile.
A `diff` problem requires a full 40-character SHA because Git can fetch an unreachable commit only by its full SHA.
The runner validates the whole file before creating any worktree or calling any model.

`is_negative: true` marks a defect that a correct review must *not* report.
Such defects are excluded from recall; scoring them for precision is future work.

## Sourcing

A problem must not **leak its answer**.
Reviewer sees must contain no hint that names the defect:
no test named after the bug, and no "this is unsound" comment.
In `files` mode, review whole files rather than line ranges narrowed to the defect.
The reviewer must reason the defect out, as it would in real use.

Prefer problems drawn from known bugs; add manual problems for failure modes the reviewer misses.

| Mode | Source | How |
|---|---|---|
| `diff` | PR-derived (preferred) | Cite the upstream commit that introduced a bug fixed later, using its full SHA and `base: HEAD^`. The expected defect is the bug the later fix removed. |
| `diff` | Manual | Create a realistic buggy commit on a sound base, publish it to a fork, and cite it by full SHA. Define the expected defect by hand. |
| `files` | Fix-anchored (preferred) | Set `commit` to the fix's parent (`<fix>^`) and review the whole files the fix touched. This also works when the introducing commit is too old or otherwise unusable. |
| `files` | Manual | Name a commit and whole files known to contain a defect, and define the expected defect by hand. Use this for issues found during inspection or never formally fixed. |

Both snapshots predate the fix, so its added guards, tests, and comments are absent.
Fix-anchored `files` mode also removes the diff as a source of hints.
Handcrafted commits may be unreferenced on a fork, but must remain fetchable by full SHA.

Use `is_negative: true` for a false-positive trap that a correct review must not report.
These cases are excluded from recall and reserved for future precision checks
(see [Grading](#grading)).

**Do not reverse a fix.**
Reversing a fixing commit deletes the guards, assertions, tests, and warning comments it added,
so the diff points straight at the answer.
So use fix-anchored `files` mode instead:
review the same bug at the fix's parent, without a diff that reveals it.

## Harness flow

[`benchmark-data/run.sh`](../benchmark-data/run.sh) runs the selected problems with the configured backend:

1. **Validate.** Load and validate all of `problems.yaml` before creating a worktree or calling a model.
2. **Reconstruct.** Check out each problem's top-level `commit` in a temporary detached worktree.
   For a `diff` problem, fetch the commit by full SHA from its remote if it is missing locally.
3. **Review.** Run the ordinary review pipeline in that worktree:
   `diff <base>` (e.g. `HEAD^`) or `files <targets>`, as specified by `review_mode`.
   Use the current ACR package and guidelines,
   with answers kept out of the review input (see [Keeping answers away from the reviewer](#keeping-answers-away-from-the-reviewer)).
4. **Grade.** With `--grade`, wait for all reviews to finish,
   then give each report and its expected defects to a separate grader run on the same backend.
5. **Aggregate.** With `--grade`, print per-problem recall and the total across all selected problems.
   Negative defects are excluded; precision scoring is future work.

Persona contexts follow the profile or the `--per-persona-context` override.
To compare combined review with one agent per persona,
run the same problems with `--per-persona-context=no` and `=yes`
(see [Execution model](execution_model.md#one-agent-per-persona)).
The runner does not automatically retry a missed defect with separate persona contexts.

`diff` mode reviews each commit's message and diff,
so a problem can include a `commit_message` defect, such as a non-imperative subject.
The grader scores it like any other expected defect.

## Keeping answers away from the reviewer

The runner shows the defects only to the grader, and only after the review report is written.
For each problem it:

1. creates a temporary detached worktree at the problem's `commit`,
   fetching it from `remote` if it is not available locally;
2. deletes the worktree's `.agents/`, `.claude/`, and historical `tools/acr/`,
   and copies in the current ACR package without its `benchmark-data/` directory;
3. points `ACR_GUIDELINE_ROOT` at the checkout where the benchmark was started,
   so the current guidelines are used rather than the historical ones;
4. runs an ordinary review with only the mode, targets, and output path;
5. removes the worktree;
6. with `--grade`, gives the report and the expected defects to the grader.

Excluding `benchmark-data/` matters:
otherwise `problems.yaml`, which holds the answers, would be readable through the source tools.

## Grading

The grader receives the report and a numbered list of expected defects,
each with its location, persona, grounding, and `expectation` as a `MATCH IF` criterion.
It returns exactly one verdict per defect:

| Verdict | Meaning |
|---|---|
| `caught` | A finding meets every material part of the expectation. |
| `partial` | A finding raises the same concern but misses a material part, or is too vague. |
| `miss` | No finding meets the expectation. |

```text
recall = caught / number of expected (non-negative) defects
```

Only `caught` counts.
A grader result that skips, repeats, or invents a defect number is rejected rather than scored.

Matching is done by a model, not by string comparison,
because a finding may describe the defect in different words,
while a nearby finding about something else must not get credit.
Recall measures the whole pipeline:
if verification wrongly retracts a true finding, the defect counts as missed.

The grader runs on the same backend as persona review stage, with the [grader prompt](../src/prompts/grader.md),
no tools, and `postprocess_model` as its model.

| File | Role |
|---|---|
| [`runner.py`](../src/benchmark/runner.py) | Problem loading and validation, the review loop, and grading. |
| [`worktree.py`](../src/benchmark/worktree.py) | Fetching commits and managing temporary worktrees. |
| [`grader.py`](../src/benchmark/grader.py) | The grader call and the recall calculation. |
| [`run.sh`](../benchmark-data/run.sh) | Command-line entry point; run from the Asterinas repository root. |
