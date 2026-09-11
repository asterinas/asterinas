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

## Writing a problem

A problem must not **leak its answer**.
The review input must contain no hint that names the defect:
no deleted guard or assertion, no test named after the bug,
no "this is unsound" comment, and in `files` mode no line range narrowed to the defect.
ACR must reason the defect out, as it would in real use.

Good sources, strongest first:

| Mode | Source | How |
|---|---|---|
| `diff` | A commit that introduced a bug fixed later | Cite the introducing commit with `base: HEAD^`. It predates the fix, so it contains none of the fix's guards, tests, or comments. |
| `diff` | A handcrafted buggy commit | Write a realistic change on a sound base. Push it to a fork and cite it by full SHA. |
| `files` | A fixed bug | Set `commit` to the fix's parent (`<fix>^`) and review the whole files the fix touched. There is no diff to leak anything, and this works even when the introducing commit is unusable. |
| `files` | A known defect never formally fixed | Name the commit and files, and describe the defect by hand. |

**Do not reverse a fix.**
Using the inverse of a fixing commit as a `diff` problem deletes exactly the guards, tests, and comments the fix added,
so the diff points straight at the answer.
Use the fix-anchored `files` problem instead.

**Commits, not patch files.**
Problems cite commits by SHA rather than storing patches,
so Git is the single source of truth and nothing can drift.
Commits on `main` are permanent.
GitHub keeps commits that no branch references
and serves them by full SHA,
so a handcrafted commit can stay on a fork even after its branch is deleted.
The cost is a dependency on that remote;
a lost commit can be recreated exactly from the same parent and change.

## Keeping answers away from the reviewer

The runner shows the defects only to the grader, and only after the review is written.
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

**Known gap.**
A detached worktree shares the Git object database and refs with the checkout where the benchmark was started.
A reviewer that can run history-wide Git queries,
such as `git log --all` or `git show <revision>:<path>`,
may reach the fixing commit or `problems.yaml` through history.
A leak-proof evaluation would need a separate repository containing only the reviewed commits,
or Git tools restricted to those commits.

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
