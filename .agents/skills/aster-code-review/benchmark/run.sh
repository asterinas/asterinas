#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

#
# run.sh — benchmark harness for aster-code-review.
# Reads `benchmark/problems.yaml`.
#
# The agent that BOTH reviews and grades is chosen by ACR_AGENT_PROFILE (required)
# — see agent_profiles/.
# The launcher (../scripts/run_agent.sh) and the skill CLI (../aster_code_review.sh)
# do the launching, so this harness names no agent
# (see spec/benchmark.md, "Agent profiles").
#
# For each problem it reconstructs the snapshot in a scratch worktree,
# runs the skill, and grades recall.
# Every problem checks out a top-level `commit` (detached HEAD);
# the mode differs only after:
#   diff  — fetch `commit` by full SHA from its `remote` if absent, worktree at it (detached);
#           review `diff <base>` (base is review_mode.diff.base, e.g. HEAD^ = the commit's parent).
#   files — worktree at `commit`;
#           review `files <targets>`.
# Each recall problem is reviewed CHEAP first (--per-persona-context=no);
# only on a miss does it escalate to the fan-out (=yes).
#
# INTEGRITY — the review agent must never see the answers:
#   * `defects`/`source` and the descriptive problem_id are ground truth for the GRADER only;
#     they never reach the reviewer.
#   * the scratch worktree path is OPAQUE (wt<N>), never the slug.
#   * overlay_skill.sh overlays the current skill but EXCLUDES benchmark/.
#
# Knobs (env vars):
#   ACR_AGENT_PROFILE    REQUIRED. a profile NAME -> agent_profiles/<name>/, or a dir path.
#   ACR_PROFILE_VARIANT  `smoke` merges the `.smoke` overlay over the base profile; unset = base.
#   MIN_RECALL       recall% gate, 0..100 (default 100).
#                    MIN_RECALL=0 is a SMOKE run:
#                    reviews only — NO grading, escalation,
#                    or precision — so it is fast and answers just "does the skill run here?".
#                    A smoke passes iff every selected problem's reviewer wrote a non-empty review;
#                    a run that errors or writes no review fails.
#                    MIN_RECALL>0 grades and gates on recall.
#   PROBLEMS         space-separated selectors;
#                    a token matches by id prefix (e.g. "0002" -> 0002-fair-weight-race).
#                    Empty/unset = all.
#   KEEP_REVIEWS     keep each problem's produced review + expected defects for inspection:
#                    a <dir>, or `1` for a temp dir (path printed).
#                    Copied post-review, so no answer key ever leaks into a review.
#   WORK             scratch dir (default: a mktemp dir, removed on exit)
#   REVIEW_CMD / GRADE_CMD / NEG_GRADE_CMD   override the agent calls (CI / mocking)
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(git -C "$HERE" rev-parse --show-toplevel)"

# --- agent launcher ---------------------------------------------------------
# ACR_AGENT_PROFILE selects the agent;
# the shared launcher (scripts/run_agent.sh) resolves and runs it,
# so this harness names no agent.
# Reviews run through the skill CLI (aster_code_review.sh);
# grader calls go straight to the launcher (grading is not a skill invocation).
SKILL="$(cd "$HERE/.." && pwd)"
ACR_CLI="$SKILL/aster_code_review.sh"
RUN_AGENT="$SKILL/scripts/run_agent.sh"
[[ -n "${ACR_AGENT_PROFILE:-}" ]] || {
    echo "run.sh: ACR_AGENT_PROFILE is required (e.g. ACR_AGENT_PROFILE=codex); run_agent.sh lists available profiles" >&2; exit 2; }

# --- scratch dirs ------------------------------------------------------------
WORK_IS_TEMP=0
if [[ -z "${WORK:-}" ]]; then WORK="$(mktemp -d)"; WORK_IS_TEMP=1; fi
mkdir -p "$WORK"
# Ground truth and the guideline tree live OUTSIDE the worktree parent,
# so a review can never reach them by walking up from its own worktree.
SPEC="$(mktemp -d)"           # per-problem expected defects (grader only)
GROOT="$(mktemp -d)"          # guidelines-only tree: current pages, no answer key
mkdir -p "$GROOT/book/src/to-contribute"
cp -r "$REPO/book/src/to-contribute/coding-guidelines" "$GROOT/book/src/to-contribute/" 2>/dev/null || true

# Optional inspection:
# keep each problem's produced review alongside its expected defects, labelled by problem_id,
# so a user can eyeball how the skill did instead of trusting the score.
# KEEP_REVIEWS=<dir> collects them there;
# KEEP_REVIEWS=1 uses a temp dir and prints it.
# Copies happen POST-review/grade,
# so nothing leaks the answer key into a review;
# the copies live outside $WORK and survive cleanup.
KEEP_DIR=""
if [[ -n "${KEEP_REVIEWS:-}" ]]; then
    if [[ "$KEEP_REVIEWS" == 1 ]]; then KEEP_DIR="$(mktemp -d)"; else KEEP_DIR="$KEEP_REVIEWS"; mkdir -p "$KEEP_DIR"; fi
fi
keep_reviews() { # <id> <wt> — copy any produced review + expected defects into KEEP_DIR/<id>/
    [[ -n "$KEEP_DIR" ]] || return 0
    local id="$1" wt="$2" dst="$KEEP_DIR/$id"; mkdir -p "$dst"
    [[ -s "$wt.off.md" ]]    && cp "$wt.off.md"    "$dst/review.md"          # combined / smoke review
    [[ -s "$wt.on.md" ]]     && cp "$wt.on.md"     "$dst/review-fanout.md"   # escalated fan-out review
    [[ -s "$wt.review.md" ]] && cp "$wt.review.md" "$dst/review.md"          # precision-problem review
    [[ -f "$SPEC/$id.defects.txt" ]]   && cp "$SPEC/$id.defects.txt"   "$dst/expected-defects.txt"
    [[ -f "$SPEC/$id.negatives.txt" ]] && cp "$SPEC/$id.negatives.txt" "$dst/expected-negatives.txt"
}

cleanup() {
    while IFS= read -r wt; do
        [[ "$wt" == "$WORK"/* ]] && git -C "$REPO" worktree remove --force "$wt" 2>/dev/null || true
    done < <(git -C "$REPO" worktree list --porcelain | awk '/^worktree /{print $2}')
    git -C "$REPO" worktree prune 2>/dev/null || true
    rm -rf "$SPEC" "$GROOT"
    [[ "$WORK_IS_TEMP" -eq 1 ]] && rm -rf "$WORK"
}
trap cleanup EXIT

# 0.
# Schema-validate; fail closed (never run on a malformed suite).
"$HERE/validate_problem_yaml.sh" >&2 || { echo "run.sh: problems.yaml failed validation; aborting" >&2; exit 2; }

# Emit the per-problem index
# AND write each problem's ground truth (defects / negatives) to $SPEC.
# Those files feed the grader only — never the reviewer.
# Index line (TAB-separated): <id> <mode> <checkout> <remote> <arg> <n_real> <n_negative>
#   checkout = commit (the snapshot to check out, both modes)
#   remote   = fetch URL for `commit` (default upstream; only consumed in diff mode)
#   arg      = diff base ref (diff, e.g. HEAD^) | space-joined file targets (files)
emit() {
  python3 - "$HERE/problems.yaml" "$SPEC" <<'PY'
import sys, os, yaml
docs = yaml.safe_load(open(sys.argv[1])); spec = sys.argv[2]
DEFAULT_REMOTE = "https://github.com/asterinas/asterinas"
def block(d, n):
    t = d["target"]; loc = t.get("path") or ("<" + t["kind"] + ">")
    if t.get("lines"): loc += " lines " + str(t["lines"])
    desc   = " ".join(str(d["desc"]).split())
    expect = " ".join(str(d["expectation"]).split())
    # 'defect:' is context; 'MATCH IF:' is the criterion the grader keys on.
    return (f"{n}. location: {loc} (persona: {d['persona']}, grounding: {d['grounding']}, severity: {d['severity']})\n"
            f"   defect: {desc}\n"
            f"   MATCH IF: {expect}")
index = []
for p in docs:
    pid, rm = p["problem_id"], p["review_mode"]
    reals = [d for d in p["defects"] if not d.get("is_negative")]
    negs  = [d for d in p["defects"] if d.get("is_negative")]
    with open(os.path.join(spec, pid + ".defects.txt"), "w") as f:
        f.write("# Expected defects\n\n" + "\n\n".join(block(d, i+1) for i, d in enumerate(reals)) + "\n")
    if negs:
        with open(os.path.join(spec, pid + ".negatives.txt"), "w") as f:
            f.write("# Must NOT be flagged (false-positive traps)\n\n" + "\n\n".join(block(d, i+1) for i, d in enumerate(negs)) + "\n")
    co = p["commit"]                                   # the snapshot every problem checks out
    remote = p.get("remote", DEFAULT_REMOTE)           # where to fetch `commit` (diff mode only)
    if "diff" in rm:
        mode, arg = "diff", rm["diff"]["base"]         # arg carries the base ref (e.g. HEAD^)
    else:
        mode, arg = "files", " ".join(rm["files"])
    index.append("\t".join([pid, mode, co, remote, arg, str(len(reals)), str(len(negs))]))
print("\n".join(index))   # all ground-truth files written before any line is emitted
PY
}

default_review() { # <worktree> <out> <skill-args>
    "$HERE/overlay_skill.sh" "$1"        # current skill into the worktree; excludes benchmark/
    # Guidelines come from GROOT (not the worktree),
    # so the historical diff stays clean.
    # Reviews run through the skill CLI — the one blessed way to invoke the skill.
    ( cd "$1" && ACR_GUIDELINE_ROOT="$GROOT" "$ACR_CLI" $3 "$2" --overwrite )
}
default_grade() { # <defects-file> <review>
    "$RUN_AGENT" "You are grading a code review. The expected defects are in $1; the \
produced review is $2. Each expected defect gives a 'defect:' description for context \
and a 'MATCH IF:' criterion. For each expected defect, decide whether ANY comment in \
the review satisfies its MATCH IF criterion at the stated code location (wording may \
differ). Respond with ONLY two space-separated integers, caught then total, and \
nothing else (for example: 1 2)."
}
default_neg_grade() { # <negatives-file> <review>
    "$RUN_AGENT" "The items in $1 are false-positive traps that a correct review must \
NOT raise as real defects. Read the review $2. Output ONLY PASS (none raised) or \
FAIL (at least one raised)."
}
REVIEW_CMD="${REVIEW_CMD:-default_review}"
GRADE_CMD="${GRADE_CMD:-default_grade}"
NEG_GRADE_CMD="${NEG_GRADE_CMD:-default_neg_grade}"

# A selector token matches a problem if its id equals the token or begins with it,
# so "0002" selects 0002-fair-weight-race (numbers are stable; slugs get reworded).
selected() { # <id>
    [[ -z "${PROBLEMS:-}" ]] && return 0
    local id="$1" tok
    for tok in $PROBLEMS; do [[ "$id" == "$tok" || "$id" == "$tok"* ]] && return 0; done
    return 1
}

# Prints "PROD" (smoke: a review was written),
# "OFF c t"/"ON c t" (recall),
# or "NEG PASS|FAIL" (pure-precision);
# non-zero on a setup/review failure.
# Opaque <wt>.
run_one() { # <wt> <id> <mode> <co> <remote> <arg> <n_real> <n_neg>
    local wt="$1" id="$2" mode="$3" co="$4" remote="$5" arg="$6" nreal="$7" nneg="$8" skillargs off on c t out base
    rm -rf "$wt"
    if [[ "$mode" == diff ]]; then
        # The change under review IS the commit.
        # Fetch it by full SHA if absent
        # (PR-derived commits are already on upstream main;
        # synthetic ones are dangling on the fork),
        # check it out detached,
        # and review `diff <base>` ($arg, e.g. HEAD^ — resolved in the worktree).
        git -C "$REPO" cat-file -e "${co}^{commit}" 2>/dev/null \
            || git -C "$REPO" fetch --no-tags "$remote" "$co" >/dev/null 2>&1 || return 1
        git -C "$REPO" worktree add -f --detach "$wt" "$co" >/dev/null 2>&1 || return 1
        base="$(git -C "$wt" rev-parse "$arg" 2>/dev/null)" || return 1
        skillargs="diff $base"
    else
        git -C "$REPO" worktree add -f --detach "$wt" "$co" >/dev/null 2>&1 || return 1
        skillargs="files $arg"
    fi

    # SMOKE (MIN_RECALL=0): the only question is "did the reviewer run and write a review?"
    # — one combined pass, then NO grading, escalation, or precision check
    # (all of which are about quality/recall, which a smoke does not judge).
    # This is what keeps a smoke fast:
    # half the agent calls, and no flaky low-effort grader.
    if [[ "$MIN_RECALL" -eq 0 ]]; then
        out="$wt.off.md"; rm -f "$out"
        "$REVIEW_CMD" "$wt" "$out" "$skillargs --per-persona-context=no" >&2 || return 1
        [[ -s "$out" ]] || return 1
        printf 'PROD\n'; return 0
    fi

    if [[ "$nreal" -eq 0 && "$nneg" -gt 0 ]]; then     # pure-precision problem
        out="$wt.review.md"; rm -f "$out"
        "$REVIEW_CMD" "$wt" "$out" "$skillargs --per-persona-context=yes" >&2 || return 1
        [[ -s "$out" ]] || return 1
        printf 'NEG %s\n' "$("$NEG_GRADE_CMD" "$SPEC/$id.negatives.txt" "$out")"
        return 0
    fi

    # Recall: cheap combined mode first, escalate to fan-out on a miss.
    # (A problem mixing real and negative defects is recall-graded only;
    #  pure-precision problems take the NEG branch above.
    #  No current problem mixes the two.)
    local df="$SPEC/$id.defects.txt"
    off="$wt.off.md"; rm -f "$off"
    "$REVIEW_CMD" "$wt" "$off" "$skillargs --per-persona-context=no" >&2 || return 1
    [[ -s "$off" ]] || return 1
    read -r c t <<<"$("$GRADE_CMD" "$df" "$off")"
    if [[ "${c:-}" =~ ^[0-9]+$ && "${t:-}" =~ ^[0-9]+$ && "$c" -eq "$nreal" && "$nreal" -gt 0 ]]; then
        printf 'OFF %s %s\n' "$c" "$nreal"; return 0
    fi
    on="$wt.on.md"; rm -f "$on"
    "$REVIEW_CMD" "$wt" "$on" "$skillargs --per-persona-context=yes" >&2 || return 1
    [[ -s "$on" ]] || return 1
    printf 'ON %s\n' "$("$GRADE_CMD" "$df" "$on")"
}

MIN_RECALL="${MIN_RECALL:-100}"       # recall gate; MIN_RECALL=0 (smoke) reviews only — no grading (run_one)
total_caught=0 total_defects=0 problems=0 off_ok=0 escalated=0 neg_pass=0 neg_total=0 n=0 harness_errors=0 produced=0
while IFS=$'\t' read -r id mode co remote arg nreal nneg <&3; do
    selected "$id" || continue
    n=$((n + 1)); wt="$WORK/wt$n"          # OPAQUE worktree path — never the slug
    if ! result="$(run_one "$wt" "$id" "$mode" "$co" "$remote" "$arg" "$nreal" "$nneg")"; then
        keep_reviews "$id" "$wt"
        printf '%-34s  ?  (harness error — setup/review failed)\n' "$id"; harness_errors=$((harness_errors + 1)); continue
    fi
    keep_reviews "$id" "$wt"
    case "$result" in
        PROD)                                    # smoke: reviewer ran + wrote a review
            produced=$((produced + 1)); printf '%-34s produced ✓\n' "$id" ;;
        NEG\ *)
            verdict="${result#NEG }"; neg_total=$((neg_total + 1))
            [[ "$verdict" == *PASS* ]] && neg_pass=$((neg_pass + 1))
            printf '%-34s precision %s\n' "$id" "$verdict" ;;
        OFF\ *|ON\ *)
            tier="${result%% *}"; read -r caught grader_total <<<"${result#* }"
            if ! [[ "${caught:-}" =~ ^[0-9]+$ && "${grader_total:-}" =~ ^[0-9]+$ ]]; then
                printf '%-34s recall  ?/?  (unparseable grader output: %q)\n' "$id" "$result"; harness_errors=$((harness_errors + 1)); continue
            fi
            if [[ "$caught" -gt "$nreal" ]]; then
                printf '%-34s recall  ?/?  (grader caught %s > expected %s)\n' "$id" "$caught" "$nreal"; harness_errors=$((harness_errors + 1)); continue
            fi
            problems=$((problems + 1)); total_caught=$((total_caught + caught)); total_defects=$((total_defects + nreal))
            if [[ "$tier" == OFF ]]; then off_ok=$((off_ok + 1)); label=combined; else escalated=$((escalated + 1)); label=fan-out; fi
            printf '%-34s recall %s/%s [%s]\n' "$id" "$caught" "$nreal" "$label" ;;
        *)  printf '%-34s recall  ?/?  (unexpected: %q)\n' "$id" "$result"; harness_errors=$((harness_errors + 1)) ;;
    esac
done 3< <(emit)

echo "----"
[[ -n "$KEEP_DIR" ]] && echo "reviews kept for inspection in: $KEEP_DIR  (per problem: review.md + expected-defects.txt)"
if [[ "$MIN_RECALL" -eq 0 ]]; then     # smoke: reviews only, no grading
    printf 'smoke: %s/%s reviews produced; harness errors: %s\n' "$produced" "$n" "$harness_errors"
    # Pass iff every attempted problem produced a review (no harness errors) and at least one ran
    # — the smoke's whole question, "does the skill run on this agent?".
    [[ "$harness_errors" -eq 0 && "$produced" -gt 0 ]]
else
    recall_pct=0
    [[ "$total_defects" -gt 0 ]] && recall_pct=$(( 100 * total_caught / total_defects ))
    printf 'recall: %s/%s (%s%%, gate >=%s%%) across %s problems; per-persona-context: %s combined, %s fan-out; precision: %s/%s clean; harness errors: %s\n' \
        "$total_caught" "$total_defects" "$recall_pct" "$MIN_RECALL" "$problems" "$off_ok" "$escalated" "$neg_pass" "$neg_total" "$harness_errors"
    # Gate: recall% >= MIN_RECALL,
    # every negative clean, no harness error, >=1 defect measured.
    pass=1
    [[ "$harness_errors" -gt 0 ]] && pass=0
    [[ "$neg_pass" != "$neg_total" ]] && pass=0
    [[ "$recall_pct" -lt "$MIN_RECALL" ]] && pass=0
    [[ "$total_defects" -eq 0 ]] && pass=0
    [[ "$pass" -eq 1 ]]
fi
