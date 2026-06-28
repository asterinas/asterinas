# Benchmark & Evaluation

A review skill is only as trustworthy as the evidence that it finds defects.
The benchmark is a suite of **review problems** with known defects
— some a Git change (`diff` mode),
some existing code at a snapshot (`files` mode);
the headline metric today is **recall**
— the fraction of those defects the skill surfaces.
It measures *review quality* and needs the model;
the deterministic scripts underneath (and this suite's schema) are verified separately by the model-free tests in [`tests/`](../tests/),
which keeps each check focused on one kind of failure.

## The near-term goal: 100% recall on one configuration

Comparing agents, models, and effort levels to find a "good enough" configuration is premature before the skill works well at all.
The near-term goal is narrower and more useful:
pin **one reference configuration
— Claude Code, Opus, high effort (`agent_profiles/claude/`)
— and drive it to 100% recall** on the problem set,
by improving the two things we control
— the **harness** (orchestration, persona prompts, verification, consolidation) and the **coding guidelines** themselves.
(The harness itself is agent-agnostic
— any agent can be run via `ACR_AGENT_PROFILE`; see [*Agent profiles*](#agent-profiles)
— so this is a choice of reference, not a hard-coding.)

With a small suite this bar is realistic,
and every miss is an actionable signal:
a defect the reference config fails to surface points at a guideline that is missing,
vague, or mis-scoped, or at a harness weakness
— and fixing that is how the guidelines and the skill improve.
A recall *matrix* across other agents,
models, and efforts is a **later** exercise,
meaningful only once the reference config clears 100%.
Near-term, the benchmark is an **optimization signal**, not a leaderboard.

## The problem file

The whole suite lives in one queryable file
— `benchmark/problems.yaml`,
a YAML sequence of problems — with each `diff`-mode problem's change referenced by the full SHA of a commit on GitHub,
not stored as a local patch (see [*Why commit references, not local patches*](#why-commit-references-not-local-patches)):

```
benchmark/
├── problems.yaml             # the suite (schema below); every problem cites a checkout `commit`
├── validate_problem_yaml.sh  # model-free schema check (run by run.sh and the tests)
├── overlay_skill.sh          # drops the current skill into a scratch worktree, minus the answers
└── run.sh                    # the harness
```

A single file — not a directory per problem
— so the suite can be **viewed, searched, and queried**.
For example, every problem the Security persona covers:

```sh
yq '.[] | select(.defects[].persona == "security") | .problem_id' benchmark/problems.yaml
```

### Schema

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

Prose fields (`source`, `desc`, `fix`, `expectation`) are YAML block scalars and may use Markdown `backticks` around code (literal text in YAML).
**`desc` and `expectation` are split on purpose:** `desc` describes the bug (context the grader and a human reader need),
while `expectation` is the *explicit criterion the grader keys on*
— "a comment matches iff it meets the `expectation`"
— phrased "A reviewer should …".
Keep `expectation` a crisp, checkable criterion, not a paraphrase of `desc`.
`severity` is informative only (never gated);
it uses the same `critical|major|minor|nit` vocabulary the review comments render,
so the two stay comparable.

`validate_problem_yaml.sh` enforces the invariants (model-free, run by `make check` and as a [`tests/`](../tests/) case):
`problem_id` unique (its numeric part too, since the harness/CLI select by number);
a top-level `commit` on every problem — a full-40-char SHA in `diff` mode,
any commit-ish in `files` mode — with an *optional* `remote` (default upstream);
exactly one of `diff`/`files`; `diff` carries a required `base` ref (e.g. `HEAD^`) and no other keys;
`files` is non-empty; no obsolete `base_commit`;
≥1 defect, each with `desc`,
`expectation`, and a `severity` from the enum;
`target.path` iff `kind: file`;
`fix` iff not `is_negative`; enums from the known sets.
A `diff` problem is a single commit reviewed as its diff against its parent,
with its message riding along
— which `diff` mode reviews (per-commit intent and commit hygiene).
The `commit_message` kind targets that message;
`whole_change` (a cross-cutting finding) is reserved for later.

### Why commit references, not local patches

An earlier design stored each `diff` problem's change as a local mbox under `patches/`
— a **redundant copy** of a commit that already lives on GitHub, free to drift from it.
We drop it: a `diff` problem now names the commit by full 40-char SHA plus a `remote` to fetch it from.
Two facts about GitHub — both verified directly — make the copy unnecessary:

- **A full-SHA fetch works even for an unreachable, force-pushed-over commit.**
  `git fetch <remote> <40-char-sha>` returns the object.
  (An *abbreviated* SHA does not — git reads it as a ref name — so the schema requires the full 40 characters.)
- **GitHub does not routinely garbage-collect such commits.**
  Its docs ([*Removing sensitive data from a repository*][gh-sensitive]) state that after a force-push the commits "may still be accessible"
  — "directly via their SHA-1 hashes" and "through any pull requests that reference them"
  — and that actually *removing* them takes a support ticket to "run a garbage collection on the server."
  Unreachable data is normally **kept**.

So neither problem kind needs a local copy.
**PR-derived** commits are ancestors of `asterinas/asterinas`'s `main`
— permanent by construction,
and already in every clone — so they cite the upstream `remote`.
**Synthetic** commits (a handcrafted regression, a bad commit message) belong to no branch:
they are pushed to a fork and left **dangling**
— exactly the "overridden during a PR" case above
— then cited by SHA against that fork,
where they stay fetchable by full SHA (verified after the branch is deleted).
The cost is a network dependency,
and that the synthetic commits live in a fork rather than mainline;
were that fork to vanish they can be re-hosted (reconstruction is deterministic — same base + change → same SHA) or pinned harder with a tag or a referencing PR.

[gh-sensitive]: https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/removing-sensitive-data-from-a-repository

## Sourcing

A problem must never **leak its own answer**.
Whatever the reviewer sees — a diff in `diff` mode,
a file in `files` mode — must contain no hint that names the defect:
not a deleted guard or assert,
not a test titled after the bug,
not a "this is unsound" comment,
and (in `files` mode) not a line range narrowed to the defect.
The reviewer has to *reason* the defect out, exactly as it would in real use.

**`diff`-mode sources:**

1. **PR-derived (the strongest signal).**
   A later commit that *fixes* a bug an earlier commit introduced is an ideal problem:
   ideally the earlier change's review would have caught it.
   The problem cites the **introducing commit** by SHA (its `remote` is upstream),
   reviewed as its diff against its parent,
   and the expected defect is the bug the fix removed.
   Because it is the *introducing* commit,
   it cannot contain the fix's later guards,
   tests, or warning comments — so the diff never hints at its own defect.
2. **Manual.**
   A handcrafted buggy commit (a realistic-looking change onto a sound base + the expected defect),
   added when a developer hits a failure mode the skill misses.
   It is hosted as a **dangling** commit on a fork and cited by SHA;
   it must read like a real change and must not include any test or comment that names the defect.

**`files`-mode sources:**

1. **Fix-anchored (the strongest signal).**
   For a known fixed bug, set `commit = fixed_by^` (the fix's parent) and review the **whole** file(s) the fix touched.
   The bug is present, but the fix's guards,
   asserts, tests, and explanatory comments do not exist yet
   — and there is no diff at all
   — so nothing can leak the answer;
   the reviewer must audit the code cold.
   This works even when the introducing commit is ancient or otherwise unusable (we anchor on the fix, not the introducer),
   which makes it the natural way to recover bugs `diff` mode cannot.
2. **Manual.**
   A `commit` plus named files known to contain a defect,
   with a handcrafted expected defect
   — for a quality issue never formally "fixed," or one a developer finds while scanning.

A defect may be marked **negative** (`is_negative: true`):
a correct review must *not* raise it.
Negative defects guard precision — see *Scoring*.

**Why not reverse-of-fix?**
An earlier version of this suite,
when a clean introducing commit was unavailable,
set the base to the *fixing* commit and used the inverse of the fix as the patch.
We dropped it: inverting a fix deletes the very guards,
asserts, tests, and "this is unsound" comments the fix added,
so the diff *handed the reviewer the answer* and resembled no change a human would write.
**Fix-anchored `files` mode is the honest replacement**
— the same bugs, anchored on the same fixes,
but reviewed as whole files with no diff,
so there is nothing to leak.

## Initial problem set

Mostly mined from real Asterinas history (all SHAs verified);
the two synthetic problems (0007–0008) are hosted as dangling commits (see [*Why commit references, not local patches*](#why-commit-references-not-local-patches)).
The `diff`-mode set exercises the **Development** and **Security** personas,
and — via a commit message — **Maintainability**:

| # | Slug | Mode | Defect & class | Persona · grounding |
|---|---|---|---|---|
| 0002 | fair-weight-race | diff | ad-hoc two-atomic lock-free weight update loses concurrent updates (data race) | Development · `careful-atomics` |
| 0004 | semop-dead-timer-retain | diff | timeout timer dropped at scope end (never fires) **and** `retain` removes *all* of a PID's ops — **two** defects | Development · `raii` + "Over-broad removal" |
| 0005 | virtio-blk-flush-desc-count | diff | flush enqueues 2 descriptors but accounts for 1 → virtqueue exhaustion (off-by-one) | Development · "Off by one" |
| 0007 | getcwd-erange | diff | `min()`-clamps a small user buffer and truncates instead of returning `ERANGE` (missing validation) | Security · `validate-at-boundaries` |
| 0001 | mprotect-merge-unwrap | diff | merge absorbs a not-yet-processed mapping → reachable `remove().unwrap()` panic, triggerable from userspace via an `mprotect` `MAY_*` bit (DoS) | Development · "Reachable panic" |
| 0008 | noncompliant-commit-message | diff | commit subject "made some changes to getcwd" is vague and past-tense, not imperative (commit hygiene) | Maintainability · `imperative-subject` |

Problem 0004 carries **two** expected defects in one change (it tests multi-defect recall).
Problems 0007 and 0008 are sourced **manually**
— 0007 a handcrafted regression of a real `getcwd` bug,
0008 a commit with a deliberately non-compliant subject (it exercises commit-message review)
— while the other `diff` problems are PR-derived.

**Restoring coverage via `files` mode.**
Two bugs dropped when reverse-of-fix was killed return as **fix-anchored `files`-mode** problems
— reviewing the touched file at `fixed_by^`,
where the bug is present and nothing names it:

| # | Slug | Mode | Defect & class | Persona · grounding |
|---|---|---|---|---|
| 0006 | trapframe-pad-alignment | files | `TrapFrame` not a multiple of 16 bytes → `%rsp` misalignment on trap entry (ABI / soundness) | Hardware · `16b-align-rsp-before-call` |
| 0003 | semop-timeout-toctou | files | status re-checked then acted on across a wait → spurious `EAGAIN` (TOCTOU) | Development · `atomic-critical-sections` |

0006 is what **restores Hardware-persona coverage**,
and these two are the suite's first `files`-mode problems
— proof that the leak-proof,
fix-anchored substrate works.

**0001 was thought negative — until verification proved it real.**
It was first filed as a precision test:
the `remove(...).unwrap()` *looked* like a reachable panic,
but a perms-equality `continue` guard appears to skip any mapping a merge could absorb.
The skill's verification step refuted that reasoning:
`mprotect` builds `VmPerms` with `from_bits_truncate` and never masks the `MAY_*` bits,
so `mprotect(p, len, PROT_READ | 0x8)` sets a `MAY_READ` bit that defeats the perms-equality skip
— the absorbed, not-yet-processed neighbour is then `remove().unwrap()`ed and the kernel panics.
So 0001 is a genuine userspace-reachable panic (a DoS),
`c7b633e9b` is a real fix, and it now counts as a **recall** problem.
(It also points at a real Asterinas bug: `mprotect` should mask user `prot` to `ALL_PERMS`.)
This is the benchmark working as intended
— the skill's own verification corrected a mislabel.

So the suite is **8 recall problems (9 expected defects)** — 6 `diff` and 2 `files`.
The 100%-recall gate is over the 9 recall defects.
There is no `is_negative` defect at the moment (0001 was reclassified);
precision is tracked via the "extra"-findings report until negative defects are added (see *Scoring*).

## Scoring

- **Matching is LLM-graded, not string-matched.**
  For each expected defect, a grader is shown the defect (location, persona, grounding, description) and the review's comments,
  and decides whether *any* comment catches it.
  A comment can describe a defect in prose no string matcher would recognize,
  and a coincidental nearby comment must not false-match
  — both argue for a judging model with a clear rubric.
  The grader is the only non-deterministic part of scoring;
  a clear rubric and a strong grader model keep it stable.
- **Recall = caught / expected**, per problem and across the suite; target **1.0**.
  Recall also guards verification:
  a true defect a persona caught but verification wrongly retracted shows up here as a miss.
- **Precision is a guardrail today.**
  Comments matching no expected defect are reported as "extra" (they may be genuine findings or noise) so the skill cannot game recall by flagging everything.
  Negative defects (`is_negative: true`; none in the suite right now — see 0001 above) make this concrete:
  the problem fails on precision if the review raises one as a real defect.

Recall leads for now because the review's first job is to *miss as few real defects as possible*;
precision is tracked as a guardrail against gaming recall.
As the benchmark gains false-positive (negative) defects,
precision graduates into a scored objective alongside recall.

## Agent profiles

The harness names no agent.
Which agent **reviews *and* grades** is chosen by **`ACR_AGENT_PROFILE`** (required — bare `make benchmark` fails closed with the list of available profiles):
a profile **name** resolving to a directory under [`agent_profiles/`](../agent_profiles/), at the skill root.
Profiles are **skill-wide**, not benchmark-only:
the shared launcher [`scripts/run_agent.sh`](../scripts/run_agent.sh) is the one thing that turns a profile into a running agent,
and the headless CLI ([`aster_code_review.sh`](../aster_code_review.sh)) and the PR-review CI use the very same profiles and launcher (see [`interface.md`](interface.md)).
Three ship, all verified end-to-end (see *Harness flow*):
**`claude`** (the reference config — Claude Code, Opus, high effort),
**`codex`** (Codex, `gpt-5.5`, high effort),
and **`codex_workflow`** (Codex, `gpt-5.5`, high effort, API-key auth — what the PR-review CI runs).

```
agent_profiles/
├── claude/
│   ├── profile.json          # command/env/inherit
│   └── profile.smoke.json    # smoke overlay: effort -> low
├── codex/
│   ├── profile.json
│   ├── config.toml           # native codex config (model/effort/sandbox)
│   └── config.smoke.toml     # smoke overlay: model_reasoning_effort -> low
└── codex_workflow/           # the PR-review CI profile
    ├── profile.json          # no `inherit` — auth is the OPENAI_API_KEY secret
    └── config.toml           # gpt-5.5, high, danger-full-access (container is the sandbox)
```

`profile.json` is a small manifest,
run **without a shell** (a prompt full of backticks, quotes, and newlines is carried as a single argv token).
Its fields:

| Field | Meaning |
|---|---|
| `command` | argv array (REQUIRED); `{prompt}`/`{workdir}`/`{home}` substituted per call. |
| `env` | environment to set (e.g. `CODEX_HOME`); `{workdir}`/`{home}` substituted. |
| `inherit` | files copied in from *outside* the profile — e.g. the agent's real `auth.json` — so a relocated, sandboxed home stays authenticated. |

By **convention** a `config.toml` in the profile dir is seeded into `{workdir}/config.toml` (no `config` key needed — `env`/`inherit` stay explicit because they are the genuinely agent-specific bits).
The launcher `run_agent.sh` makes the private `{workdir}`,
seeds the config, applies `env`,
copies `inherit`, and runs `command` with `{prompt}`
— for every model call (the reviewer via the skill CLI, the graders directly).
This is **whole-stack**: one profile drives every model call,
so a run needs only that one agent installed (a Codex-only or Claude-only machine can run the whole benchmark).
The trade-off is that recall is not comparable *across* profiles (the grader changes with the reviewer)
— fine, because comparing configs is a later goal;
a smoke test just asks "does this agent work?"

**Model and effort are pinned
— and they propagate into the persona passes**,
because a profile's job is to document one exact,
reproducible configuration (what CI runs) and to record which agent/model/effort actually works.
Each agent propagates through its *own* native mechanism
— no skill changes, no benchmark-invented env vars:

- **Claude** — `--model`/`--effort` are session-level, so the Task passes inherit them.
- **Codex** — model/effort live in a `config.toml` seeded into a private `CODEX_HOME`;
  since `CODEX_HOME` is an env var,
  every nested `codex exec` the skill spawns for a persona inherits it and reads the same config.
  (Relocating `CODEX_HOME` is exactly why `inherit` exists — codex keeps its login token in `CODEX_HOME/auth.json`, so the profile copies the real one in.)

Pinning has one caveat worth stating:
model *availability* is account-specific (a ChatGPT-account login serves `gpt-5.5`; an API key may serve `gpt-5-codex`),
so a shipped profile's `model` may need adjusting for another account
— that's the profile doing its job of naming an exact config, not a bug.

### Benchmark vs. smoke

`make benchmark ACR_AGENT_PROFILE=<name>` runs the **full suite at `MIN_RECALL=100`** on the **base** profile
(the gate is a `MIN_RECALL` knob — the PR-review CI runs it *informational* at `MIN_RECALL=1`, since `codex_workflow` is not the reference config).
`make smoke ACR_AGENT_PROFILE=<name>` runs a **2-problem subset at `MIN_RECALL=0`** on the **`.smoke` overlay**
— tuned three ways for speed:

- **Faster config (`.smoke` overlay).**
  With `ACR_PROFILE_VARIANT=smoke`,
  `run.sh` shallow-merges `profile.smoke.json` over `profile.json` and `config.smoke.toml` over `config.toml`
  — a smoke key wins.
  Every overlay drops reasoning **effort to `low`**;
  the *model* is then tuned **per agent by what was measured to be faster**,
  not assumed: **Codex → `gpt-5.4-mini`** (a lighter model roughly halved its wall-clock),
  but **Claude stays `opus`** — there,
  lighter models (Sonnet, Haiku) were consistently *slower*,
  because driving the skill's multi-turn agentic pipeline is turn-bound and Opus finishes it in fewer turns.
  (So "weaker = faster" held for Codex and reversed for Claude — worth re-measuring per agent, not porting the assumption.)
- **Two problems, both modes.**
  `0002` (diff-code) + `0006` (files)
  — the two review modes and both reconstruction paths.
  (`commit_message` review is a diff-mode sub-feature, left to the full suite.)
- **No grading (nor escalation).**
  `MIN_RECALL=0` runs **one combined review per problem and stops**
  — no grade call, no fan-out escalation,
  no precision check, all of which judge *quality* (which a smoke does not).
  That **halves the agent calls** (reviews only, no graders) and deletes the flaky low-effort grader entirely.
  (Grading, escalation, and fan-out are all still exercised by the full benchmark.)

A smoke **passes iff every selected problem's reviewer wrote a non-empty review** (and none errored)
— that is the whole question,
"does the skill *run* on this agent?".
Recall and precision never enter a smoke's verdict; they belong to the full benchmark.

**`PROBLEMS`** narrows either target
— space-separated selectors matched by **id prefix** (`PROBLEMS="0002 0006"`; `0002` matches `0002-fair-weight-race`).
The everyday use is iterating on one new problem while tuning a guideline to recall it.

**`KEEP`** keeps the produced reviews for you to inspect rather than trust the score:
`make benchmark ACR_AGENT_PROFILE=… KEEP=<dir>` (or `KEEP=1` for a printed temp dir) copies each problem's review next to its expected defects
— `<dir>/<problem_id>/review.md` and `expected-defects.txt`
— so you can read review-vs-expected side by side.
The copies happen *after* each review is graded,
so they never leak the answer key into a review.

## Harness flow

`benchmark/run.sh` runs, for the agent named by `ACR_AGENT_PROFILE`:

0. **Validate** `problems.yaml` against the schema (`validate_problem_yaml.sh`) and **fail closed**
   — never score a malformed suite.
1. **Lay out the ground truth**
   — write each problem's `defects` (and any negatives) to scratch files the *grader* will read.
   These never go near the reviewer.
2. **Reconstruct the snapshot**
   — every problem checks out its top-level `commit` in a detached scratch worktree
   (for a `diff` problem, fetch it by full SHA from its `remote` first if absent, then review as `diff <base>`, e.g. `HEAD^`;
   for a `files` problem, just review the named targets at that checkout).
   Either way the code under review is now the working tree — the skill's one rule.
3. **Review, cheap first** — run the skill in the worktree via `ACR_AGENT_PROFILE`'s agent (`diff <base>` or `files <targets>`) with `--per-persona-context=no`;
   only if it misses a defect *and* `MIN_RECALL>0` escalate to `=yes` and review again (a smoke run, `MIN_RECALL=0`, skips this).
4. **Grade** — the same profile's agent sees the review and that problem's `defects`,
   and reports `caught / expected` (a precision pass/fail for any negative defects).
5. **Aggregate** — per-problem recall and the suite total;
   the gate passes iff recall% ≥ `MIN_RECALL`,
   every negative is clean, and **no** problem hit a harness/validity error.
   The per-problem combined-vs-fan-out outcome is the labelled data a future `--per-persona-context=auto` heuristic is trained on (see [`execution_model.md`](execution_model.md)).

**Integrity — the reviewer never sees the answers.**
A recall benchmark is worthless if the review agent can read the expected defects,
so the harness guards three leak vectors:
`defects` and `source` go **only** to the grader;
the scratch worktree path is **opaque** (never the descriptive `problem_id`);
and `overlay_skill.sh` — which drops the current skill into the worktree so it reviews with up-to-date guidelines
— **excludes `benchmark/`**,
or `problems.yaml` (the entire answer key) would sit in the tree under review.
The reviewer's inputs derive solely from the reconstructed worktree and its `review_mode` arguments.

The harness fails closed: a setup or review failure (a commit that won't fetch, a review that produces nothing) is a harness error,
never a fabricated score.
The same runs double as a **regression guard**
— a guideline or harness edit must not drop recall on a problem that previously passed.

**Commit-message review.**
`diff` mode reviews the **commit series** (each commit's message + diff),
so a `diff` problem can carry a `commit_message` defect
— a commit whose message violates a convention (`imperative-subject`, `atomic-commits`, …)
— graded like any other.
(`whole_change`, a cross-cutting finding spanning the series, is reserved for later.)

**Verified agents.**
Both shipped profiles pass the smoke set end-to-end:
`claude` and `codex` each produce valid,
on-target reviews — Codex across **both** the combined pass *and* the nested-`codex exec` fan-out.
That is exactly what a smoke test proves
— that the agent-agnostic skill *runs* on a second agent
— as distinct from the 100%-recall bar the reference config is separately driven to.
