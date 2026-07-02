# Tests

Integration tests for `aster-code-review`'s **deterministic machinery**
— the `resolve-target.sh`, `build-pass-prompt.sh`,
and `assemble-review.sh` scripts,
plus a schema check of the benchmark's `problems.yaml`.
They are model-free, fast, and self-contained:
each test case builds a throwaway Git repository or fragment set,
exercises the script, and asserts on its output and exit status.

Run the whole suite from the skill root — it delegates here:

```sh
make test
```

For granular runs, this directory has its own [`Makefile`](Makefile):

```sh
make -C tests                       # all suites (the default goal)
make -C tests test-resolve-target   # one suite
make -C tests help                  # list the discovered suites
```

Each command exits non-zero if any case fails.
Suites are auto-discovered (`test-*.sh`), so adding one needs no Makefile edit.

## Layout

One suite file per script, plus a shared library
— add a script, add a suite;
add an aspect, add a `test_*` function.

| File | What it holds |
|---|---|
| [`Makefile`](Makefile) | Auto-discovers `test-*.sh`; default goal runs all suites, `test-<suite>` runs one. |
| [`lib.sh`](lib.sh) | Assert helpers (`assert_eq`, `assert_contains`, `assert_absent`, `assert_before`), the standard Git **fixture** builder (`build_repo`), and the case runner (`run_suite`). |
| [`test-resolve-target.sh`](test-resolve-target.sh) | Cases for `scripts/resolve-target.sh`. |
| [`test-build-pass-prompt.sh`](test-build-pass-prompt.sh) | Cases for `scripts/build-pass-prompt.sh`. |
| [`test-assemble-review.sh`](test-assemble-review.sh) | Cases for `scripts/assemble-review.sh`. |
| [`test-problems-schema.sh`](test-problems-schema.sh) | Schema-validates `benchmark/problems.yaml` (via `benchmark/validate-problem-yaml.sh`). |

A suite sources `lib.sh`, defines one `test_<aspect>` function per case (and an optional `setup` that prepares a fresh `$TMP` per case),
and ends with `run_suite`.
The runner discovers every `test_*` function,
runs each in its own scratch directory,
and reports `ok` / `FAIL` per case with a summary.

## How this differs from `benchmark/`

The two are complementary and deliberately separate:

| | [`tests/`](.) | [`benchmark/`](../benchmark/) |
|---|---|---|
| Verifies | the *machinery* — change resolution and review assembly behave correctly | the *review quality* — does the skill find the planted defects? |
| Needs a model? | No (pure scripts) | Yes (persona passes + an LLM grader) |
| Speed | seconds | minutes |
| Headline | per-case `ok`/`FAIL` | recall (target 100%) |

A bug in the deterministic primitives would corrupt every review *and* every benchmark run,
so these tests are the first line of defense;
`benchmark/` then measures the judgement the scripts cannot.

## What is covered

- **`resolve-target.sh`** — the self-tokenizing argument grammar (missing mode, unknown mode/flag, lone positional, unbalanced quote each exit 2);
  `diff` mode (one base only, no range or `base..head`, merge-base→working-tree resolution, the `-dirty` head marker);
  and `files` mode (quoted paths, merged/sorted line ranges, whole-file and range excerpts, missing-file error).
- **`build-pass-prompt.sh`** — arity/validation;
  the stable-then-volatile ordering (contract → persona + inlined guideline → review input);
  guideline inlining; and the cache property
  — the prefix up to the review input is byte-identical regardless of the input (and the input body never leaks into that prefix).
- **`assemble-review.sh`** — comment rendering,
  persona grouping and file→line ordering,
  dedup within a persona vs. keeping a comment across two personas,
  YAML title escaping, the `--overwrite` refuse-to-clobber guard, and arity errors.
- **`benchmark/problems.yaml`**
  — `validate-problem-yaml.sh` schema check:
  unique ids, exactly one of `diff`/`files`,
  the named patch exists, ≥1 defect,
  per-defect `target`/`persona`/`grounding` from the known sets,
  and `fix` present iff not `is_negative`.
