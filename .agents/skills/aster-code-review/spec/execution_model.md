# Execution Model

The skill **fans out**: each activated persona reviews the code as an independent **pass** in its own clean context,
and a thin **orchestrator** ties the passes together
— resolving the review target (a Git change or a set of files) into one canonical input,
assembling the fragments, then verifying,
consolidating, and summarizing them into the review file.

```
┌────────────────────────────── ORCHESTRATOR ───────────────────────────────┐
│  1. resolve-target     -> canonical review input (diff | files)           │
│  2. activate personas  -> path-gated subset of the five                   │
│  3. fan out            -> one isolated PASS per active persona            │
│  4. collect fragments  <- each pass returns its persona's comments        │
│  5. assemble           -> group, sort, exact-dedup     [deterministic]    │
│  6. verify             -> fact-check claims; drop confident-false  [LLM]  │
│  7. consolidate        -> unify shared-root-cause fixes; keep all  [LLM]  │
│  8. write # Summary    -> synthesis over the verified, final set   [LLM]  │
│  9. write <output>                                                        │
└───────────────────────────────────────────────────────────────────────────┘

step 3-4 -- the fan-out (one isolated PASS each):
┌───────────┬───────────┬───────────┬───────────┬───────────┐
│ Maintain. │  Correct. │  Security │  Hardware │    Doc.   │
│   (pass)  │   (pass)  │   (pass)  │   (pass)  │   (pass)  │
└───────────┴───────────┴───────────┴───────────┴───────────┘
each pass reads ONLY its own persona + complete gist catalog + canonical review input,
then fetches exact rule chunks on concrete suspicion
```

Steps 1 and 5 are deterministic scripts; steps 6–8 use the model.
The orchestrator owns everything *outside* the passes
— target resolution, assembly,
verification, consolidation,
and the summary; a pass only returns its fragment and never writes the output file.

## The canonical review input

Step 1 reduces either mode to **one canonical review input** that the passes read,
so everything downstream is mode-agnostic:

- **`diff` mode** emits the **commit series**
  — `git log -p --reverse $(git merge-base <base> HEAD)..HEAD`,
  each commit's message and its diff,
  so a pass can judge per-commit intent and commit hygiene as well as the code (see [`interface.md`](interface.md)).
- **`files` mode** emits the selected file excerpts
  — the named paths' working-tree contents,
  restricted to the requested line ranges,
  each annotated with its path and line numbers so a comment can anchor to a line.

In both cases the unit of review is the emitted input,
but a pass may read the surrounding file and the wider repository for context (the working tree is right there).
Activation and every later step treat the two the same; only resolution differs.

## Fan-out: one isolated pass per persona

A pass is a pure function: `(review_input, persona_catalog, queried_rule_chunks) → comment_fragment`.
It runs in a **clean context** that initially loads only its own persona and complete gist catalog.

**Why fan out, rather than one prompt that walks all five personas?**
Because the persona guidelines exist to give a reviewer *selective exposure* (see [`coding_guidelines.md`](coding_guidelines.md)).
A single shared context would re-accumulate all five persona catalogs plus the review input on every turn,
throwing that benefit away — exactly the context bloat the persona structure was built to avoid.
Fan-out also makes a pass the unit the benchmark can score in isolation.

The combined single pass is **not rejected,
but demoted to an opt-in cheap mode** (`--per-persona-context=no`; see [`interface.md`](interface.md)).
It pays the review input and the spawn overhead once,
so it is cheaper — but the larger shared context dilutes focus and finds fewer defects,
a real recall cost the benchmark confirms (the combined pass missed a defect a focused pass caught).
So fan-out is the default: `auto` resolves to it today,
and the cheap mode is something a caller asks for.
Turning `auto` into a *measured* heuristic
— combine when it is safe, fan out when it matters
— is future work, and the benchmark is built to produce exactly that signal (see [`benchmark.md`](benchmark.md)).

**Pass prompts are assembled deterministically, ordered for cache reuse.**
Each pass prompt is built by `build_pass_prompt.sh`
— not retyped by the orchestrator
— so its prefix is byte-identical across passes and across reviews:
the **stable** blocks come first (the shared reviewer contract `pass_contract.md`, then each persona's template and complete gist catalog)
and the **volatile** review input comes last.
The orchestrator spawns the sub-agent with that exact text,
which makes the stable head a reusable prompt-cache prefix.
The most reliable win is across *successive reviews*
— the self-review loop re-running the same personas within the cache window re-pays only the input
— because *within* a single review the parallel passes race to warm the cache.
The cache-control breakpoints themselves are set by the host agent,
not the skill, so this ordering *enables* caching rather than forcing it;
the token saving is confirmed on the benchmark, not asserted.

### What a pass does internally: concern phases

A pass is not a single prompt;
it works its persona's concerns in dependency order,
reading each rule's gist first and drilling into the full rule only on a suspected violation.
The pass gathers candidate short-names for one concern phase and calls
`guideline_query.py show --expect-digest <catalog-digest> <persona> <short-name>...`
once for that batch.
The tool returns the exact authored H3 sections in catalog order;
the pass must read a rule chunk before using its short-name as a finding's grounding.
It does not query every rule preemptively,
and real in-scope defects without a matching guideline continue to use plain-language grounding.
The query command resolves the same current or benchmark-bundled corpus used to build the catalog,
so a historical worktree cannot silently supply stale rule text.
The ordered concerns are the second axis of the design (personas are parallel; concerns are sequential within a pass):

- **Maintainability:** intent & goal → design / interface fit → naming,
  comments, layout (incl. Rust-specific).
- **Development (Correctness):** trace logic for correctness & edge cases → error & resource handling (`Drop`) → concurrency (lock order, atomics) → hot-path efficiency → logging & test adequacy.
- **Security:** `unsafe` soundness → validation of untrusted input at boundaries → exploitable concurrency (use-after-free, TOCTOU).
- **Hardware:** assembly conventions → per-architecture ABI invariants.
- **Documentation:** general style → path-specific doc currency.

Independently of rule-matching,
each pass also reasons about code for defects that belong to its persona.
This is not a general bug sweep repeated by every persona:

- Maintainability owns structural and process failures.
- Development owns runtime correctness.
- Security owns adversarial and soundness failures.
- Hardware owns ABI and architecture hazards.
- Documentation owns doc style and currency.

If a suspected defect naturally belongs to another active persona,
the pass should not duplicate that investigation.
Within its owned failure modes,
a pass reports real defects even when no rule names them
— each grounded in a short plain-language description of the defect ("Off by one", "Data race", …), not the bare word `bug` —
and applies an **adversarial self-check**:
before dismissing a suspected in-scope defect as safe,
it states the concrete input or interleaving that would trigger it.
"It looks fine" is not a verdict.

## Activation: which personas run

A persona runs unless the reviewed code *provably* contains nothing in its remit.
Activation reads the set of paths under review
— the changed paths in `diff` mode,
the named paths in `files` mode:

- **Maintainability, Development, Security** run on any change that touches code.
  (Maintainability effectively always runs — its process rules, like a well-formed commit subject in `diff` mode, apply to nearly every review.)
- **Hardware** runs only when a reviewed path is assembly,
  an architecture-specific directory,
  or contains `asm!`/`global_asm!`.
- **Documentation** runs when a reviewed path is docs/book,
  a syscall-coverage (`.scml`) file,
  or a user-facing API surface.

Activation is **path/pattern-based and deterministic** — never a model "triage" step.
A model triage that wrongly drops a persona is a silent recall hole that is hard to attribute;
a deterministic gate is reproducible and lets the benchmark assert the gating itself ("Hardware should have activated here").
This is a tunable heuristic,
free to evolve, which is why it is engineered for recall first.

## Assembly: deterministic, with the model only for the summary

Assembly is split to protect recall and reproducibility:

- **Deterministic merge (pure code, step 5).**
  Gather the persona fragments,
  group them under their persona's `##` section in a fixed persona order,
  sort by file → line within each section,
  drop only *exact* textual duplicates *within a persona*, and write the frontmatter.
  No model judgement, so the body is reproducible
  — the same input yields the same comments,
  which the benchmark needs — and no finding is ever silently dropped by a merge step.
- **The model writes only the summary (step 8).**
  It synthesizes a constructive `# Summary` over the final comment set;
  it may never delete or rewrite a comment.

The accepted cost is mild redundancy:
two personas flagging one line yield two comments,
now in their separate persona sections.
The rejected alternative — a model "reconciliation" pass that dedups and merges across the whole set
— reintroduces non-determinism and a silent-drop recall hole,
which is precisely what this split avoids.
(Cross-persona overlap is instead handled, non-destructively, by consolidation below.)

## Post-merge refinement: verification, then consolidation

Fan-out buys context economy at two costs:
a lone persona can assert a false external fact it has no way to check,
and no pass can see that several comments share one fix.
The two refinement passes earn both back
— precision and coherence — *without* sacrificing recall.
They run as two separate, tightly-ruled passes rather than one combined "polish" step,
because a single pass empowered to both drop and merge is exactly the non-deterministic reconciliation the assembly step rejects;
splitting them lets each carry one narrow, separately-testable rule.

### Verification (step 6)

A fanned-out persona reasons alone over a slice of the code,
so its weakest point is a **factual premise it cannot see is wrong**
— what Linux returns for a syscall edge case,
what the System V ABI mandates,
how a Rust API actually behaves.
Verification adversarially fact-checks those load-bearing claims:
for each comment it isolates the key premise and tries to **refute** it,
re-reading the cited code and consulting authoritative sources (Linux man pages and source, POSIX, hardware manuals, the Rust reference).
Each comment gets a verdict:

- **confirmed** — the premise holds; the comment stands unchanged.
- **uncertain** — it could not be settled;
  the comment stays but is flagged `(unverified)` so a reader (or the loop) can weight it.
- **refuted** — the premise is demonstrably false;
  the comment is removed and recorded in a short *Retracted by verification* note at the foot of the file
  — never silently.

Removal requires *confident* refutation:
an unsure verifier returns `uncertain`, not `refuted`.
This is the **only** point at which a comment can leave the body,
and it removes only false positives
— raising precision without endangering a real finding.
The benchmark polices the bar directly:
a wrongly-refuted true defect resurfaces as a recall miss,
so over-eager verification shows up immediately as dropped recall.

### Consolidation (step 7)

Independent passes see independent symptoms,
but several comments often share **one root cause or one fix**
— five manual lock/unlock pairs that all want RAII,
three call sites missing the same bounds check.
Applied piecemeal, those are busywork at best and mutually conflicting at worst.
Consolidation clusters comments that share a root cause or remedy and writes a single **unified fix** for the cluster,
repointing each member's **Fix.** at it (e.g. *"Shared with the other `raii` comments: introduce a `Guard` that releases in `Drop`."*).
It **never removes a comment**
— every symptom stays visible at its own `###` location,
so recall is untouched — and it runs *after* verification,
so it never builds a unified fix around a comment that is about to be retracted.

## Agent-agnostic packaging

The skill ships as **one package** under `.agents/skills/aster-code-review/`,
discovered by Codex via the `.agents/` convention and by Claude Code via a `.claude/skills/aster-code-review` symlink
— the dual pattern this repo already uses for its other skills.
Three commitments keep its behavior identical across agents and reproducible for the benchmark:

- **Deterministic primitives are scripts, not prose.**
  Target resolution (`resolve_target.sh` — argument parsing plus emitting the canonical review input),
  guideline access (`guideline_query.py` — root resolution, corpus validation, catalogs, and exact rule chunks),
  pass-prompt assembly (`build_pass_prompt.sh` — the cache-ordered stable-then-volatile prompt),
  and review assembly (`assemble_review.sh`) are committed scripts,
  so their behavior is byte-identical across Claude,
  Codex, and the benchmark — and the benchmark therefore measures exactly what users run.
  The rejected alternative, encoding these as natural-language instructions,
  is non-reproducible across agents and models.
  Because they are pure scripts,
  they are covered by model-free integration tests in `tests/`,
  kept separate from the model-driven `benchmark/`.
  Argument parsing lives in the script (not the agent) for the same reason:
  the skill receives one raw argument string and tokenizes it deterministically (see [`interface.md`](interface.md)),
  so the parse is identical however the skill was triggered.
- **Guidelines stay single-sourced.**
  Each persona template links to its book page rather than copying rules,
  so a single rule edit updates author guidance and reviewer behavior at once.
  The rejected alternative — separate skills per agent
  — duplicates the persona logic and lets the two implementations drift;
  the whole point is one standard, one behavior.
- **One agent-specific seam.**
  The only thing that differs between agents is the primitive that "runs a persona pass in a clean context":
  Claude Code spawns a Task sub-agent per persona;
  Codex issues `codex exec` per persona (the Codex host shells out to itself, exactly as Claude issues Task calls).
  The user never types `codex exec`
  — the user-facing invocation is identical in-session on both agents.
  The benchmark selects the agent through `ACR_AGENT_PROFILE` (see [`benchmark.md`](benchmark.md)),
  and its smoke test exercises **both** seams end-to-end
  — Codex's per-persona `codex exec` fan-out is *verified* to run and return the JSON comment contract,
  not merely asserted.
- **Tasks run through `make`, which only delegates.**
  The package's root `Makefile` is a pure dispatcher:
  `make test`, `make benchmark`,
  and `make smoke` each hand off to a `Makefile` in the owning subdirectory (`tests/`, `benchmark/`).
  The root knows the directory names,
  not the task details — the suite list,
  script names, and how a benchmark runs all live behind the sub-Makefile that owns them.
