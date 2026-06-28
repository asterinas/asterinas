#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

#
# run_agent.sh — launch the ACR_AGENT_PROFILE agent with one prompt, headless,
# with NO shell (so a prompt full of backticks/quotes/newlines is safe).
#
# The shared launcher: aster_code_review.sh runs the skill through it,
# and the benchmark (run.sh) uses it for its grader calls.
# It is the ONE place that knows how to turn a profile into a running agent.
#
# Usage: run_agent.sh "<prompt>"
# Env:
#   ACR_AGENT_PROFILE    REQUIRED. a profile NAME -> agent_profiles/<name>/, or a dir path.
#   ACR_PROFILE_VARIANT  `smoke` merges the `.smoke` overlay over the base profile; unset = base.
#
# A profile dir holds profile.json (command/env/inherit)
# and, by convention, an optional config.toml seeded into a private {workdir}.
# {prompt}/{workdir}/{home} in the profile are substituted.
# Runs in the current cwd and inherits the current env PLUS the profile env.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SKILL="$(cd "$HERE/.." && pwd)"
profiles_dir="$SKILL/agent_profiles"

[[ $# -eq 1 ]] || { echo "usage: run_agent.sh \"<prompt>\"" >&2; exit 2; }
prompt="$1"

# Recursion guard.
# This launcher spawns the review agent,
# so being re-entered from *inside* a running agent
# means the agent mistakenly re-ran the skill launcher (aster_code_review.sh)
# instead of spawning a persona pass
# — an infinite agent fork bomb
# (observed as many nested `codex exec "Use the aster-code-review skill ..."`
# processes until the runner is killed).
# Fail fast.
# A legitimate persona pass is a plain `codex exec`/Task with a build_pass_prompt.sh prompt
# and never re-enters here;
# the env var is exported so it reaches the agent and any shell it runs.
if [[ -n "${ACR_AGENT_RUNNING:-}" ]]; then
    echo "run_agent.sh: refusing to re-enter the skill launcher from within a running agent (recursion). Spawn a persona pass as a plain 'codex exec'/Task with the build_pass_prompt.sh prompt, not aster_code_review.sh / run_agent.sh." >&2
    exit 3
fi
export ACR_AGENT_RUNNING=1

list_profiles() { find "$profiles_dir" -mindepth 2 -maxdepth 2 -name profile.json -printf '%h\n' 2>/dev/null | xargs -r -n1 basename | sort | tr '\n' ' '; }
[[ -n "${ACR_AGENT_PROFILE:-}" ]] || {
    echo "run_agent.sh: ACR_AGENT_PROFILE is required (e.g. ACR_AGENT_PROFILE=codex). Available: $(list_profiles)" >&2; exit 2; }
if [[ "$ACR_AGENT_PROFILE" == */* ]]; then PROFILE_DIR="$ACR_AGENT_PROFILE"; else PROFILE_DIR="$profiles_dir/$ACR_AGENT_PROFILE"; fi
[[ -f "$PROFILE_DIR/profile.json" ]] || { echo "run_agent.sh: profile not found: $PROFILE_DIR/profile.json (available: $(list_profiles))" >&2; exit 2; }
PROFILE_DIR="$(cd "$PROFILE_DIR" && pwd)"
PROFILE_SMOKE=0; [[ "${ACR_PROFILE_VARIANT:-}" == smoke ]] && PROFILE_SMOKE=1
PROFILE_WORKDIR="$(mktemp -d)"          # the {workdir}: config.toml + auth land here (e.g. CODEX_HOME)
trap 'rm -rf "$PROFILE_WORKDIR"' EXIT
declare -a PROFILE_CMD=() PROFILE_ENV=() INH_SRC=() INH_DEST=()

# Parse the (smoke-merged) profile.json into C<TAB>token | E<TAB>KEY=VAL | I<TAB>src<TAB>dest,
# and seed the (smoke-merged) config.toml into {workdir}.
# {workdir}/{home} are resolved now (static);
# {prompt} is left for the launch below.
profile_parsed="$(python3 - "$PROFILE_DIR" "$PROFILE_WORKDIR" "$HOME" "$PROFILE_SMOKE" <<'PY'
import json, os, sys
pdir, workdir, home, smoke = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4] == "1"
def load_json(p):
    if not os.path.exists(p): return {}
    try: return json.load(open(p))
    except Exception as e: sys.stderr.write(f"invalid JSON {p}: {e}\n"); sys.exit(3)
prof = load_json(os.path.join(pdir, "profile.json"))
if smoke: prof.update(load_json(os.path.join(pdir, "profile.smoke.json")))   # shallow: a smoke key wins
cmd = prof.get("command")
if not isinstance(cmd, list) or not cmd or not all(isinstance(x, str) for x in cmd):
    sys.stderr.write("profile 'command' must be a non-empty array of strings\n"); sys.exit(3)
def sub(s): return str(s).replace("{workdir}", workdir).replace("{home}", home)
for t in cmd:                                          print("C\t" + sub(t))
for k, v in (prof.get("env") or {}).items():           print("E\t" + f"{k}={sub(v)}")
for src, dest in (prof.get("inherit") or {}).items():  print("I\t" + sub(str(src)) + "\t" + sub(dest))
# config convention: seed config.toml into {workdir} if present, shallow-merging the
# smoke overlay. Top-level `key = value` scalars merge per-key (a smoke key wins);
# any `[table]` section (e.g. a `[model_providers.*]` for API-key auth) is preserved
# verbatim after the scalars — the flat merge cannot represent it. Comments are dropped
# from the seeded copy (the source keeps them); tables must follow the scalars, as ours do.
def toml_flat(p):        # top-level scalar keys only — stop at the first table header
    d = {}
    if os.path.exists(p):
        for ln in open(p):
            s = ln.strip()
            if s.startswith("["): break
            if s and not s.startswith("#") and "=" in s:
                k, v = s.split("=", 1); d[k.strip()] = v.strip()
    return d
def toml_tables(p):      # raw lines from the first table header to EOF, kept verbatim
    out, started = [], False
    if os.path.exists(p):
        for ln in open(p):
            if not started and ln.lstrip().startswith("["): started = True
            if started: out.append(ln.rstrip("\n"))
    return out
base = os.path.join(pdir, "config.toml")
if os.path.exists(base):
    cfg, tables = toml_flat(base), toml_tables(base)
    if smoke:
        smk = os.path.join(pdir, "config.smoke.toml")
        cfg.update(toml_flat(smk))
        st = toml_tables(smk)
        if st: tables = st                     # a smoke overlay with tables replaces (none today)
    with open(os.path.join(workdir, "config.toml"), "w") as f:
        for k, v in cfg.items(): f.write(f"{k} = {v}\n")
        if tables: f.write("\n" + "\n".join(tables) + "\n")
PY
)" || { echo "run_agent.sh: invalid profile: $PROFILE_DIR" >&2; exit 2; }
while IFS=$'\t' read -r tag a b; do
    case "$tag" in
        C) PROFILE_CMD+=("$a") ;;
        E) PROFILE_ENV+=("$a") ;;
        I) INH_SRC+=("$a"); INH_DEST+=("$b") ;;
    esac
done <<<"$profile_parsed"
for i in "${!INH_SRC[@]}"; do                     # inherit outside files (e.g. the agent's real auth)
    src="${INH_SRC[$i]}"; dest="$PROFILE_WORKDIR/${INH_DEST[$i]}"
    [[ -f "$src" ]] || { echo "run_agent.sh: profile 'inherit' source not found: $src (is the agent logged in?)" >&2; exit 2; }
    mkdir -p "$(dirname "$dest")"; cp "$src" "$dest"
done

declare -a argv=()
for tok in "${PROFILE_CMD[@]}"; do argv+=("${tok//\{prompt\}/$prompt}"); done
# stdin from /dev/null: the prompt is an argv token,
# and a headless agent that also reads stdin
# (e.g. `codex exec` appends piped stdin as a <stdin> block)
# must NOT swallow the caller's stdin (the benchmark loop feeds the problem list on another FD).
if [[ ${#PROFILE_ENV[@]} -gt 0 ]]; then env "${PROFILE_ENV[@]}" "${argv[@]}" </dev/null; else "${argv[@]}" </dev/null; fi
