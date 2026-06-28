#!/usr/bin/env bash
#
# assemble-review.sh — deterministic merge of persona comment fragments into the review file body.
# No LLM judgement here (see ADR-0003):
# group by persona in a fixed order,
# sort by file then line within each group,
# drop exact duplicates *within a persona*,
# write the frontmatter, and leave a <!-- SUMMARY --> placeholder for the orchestrator's LLM summary step.
#
# Usage: assemble-review.sh [--overwrite] <meta-file> <frag-dir> <output-file>
#   meta-file: lines `mode=`, `base=` (diff) or `files=` (files), `head=`,
#              `branch=`, `date=`, `title=`
#   frag-dir:  one <persona>.json per persona, each a JSON array of comments:
#       {"file","line","grounding","severity","problem","fix","diff"}
#   --overwrite: replace <output-file> if it already exists (otherwise fail).
set -euo pipefail

overwrite=0
pos=()
for a in "$@"; do
    case "$a" in
        --overwrite) overwrite=1 ;;
        --*) echo "assemble-review.sh: unknown flag: $a" >&2; exit 2 ;;
        *) pos+=("$a") ;;
    esac
done
if [[ ${#pos[@]} -ne 3 ]]; then
    echo "usage: assemble-review.sh [--overwrite] <meta-file> <frag-dir> <output-file>" >&2
    exit 2
fi
meta="${pos[0]}"; fragdir="${pos[1]}"; out="${pos[2]}"

# Refuse to clobber an existing review unless --overwrite (design §3 / ADR-0002).
if [[ -e "$out" && $overwrite -eq 0 ]]; then
    echo "assemble-review.sh: refusing to overwrite existing $out (pass --overwrite)" >&2
    exit 1
fi

python3 - "$meta" "$fragdir" "$out" <<'PY'
import json, os, sys
meta_path, fragdir, out = sys.argv[1], sys.argv[2], sys.argv[3]

meta = {}
for line in open(meta_path):
    line = line.strip()
    if "=" in line:
        k, v = line.split("=", 1)
        meta[k] = v

# Fixed persona order -> review section title.
# "Correctness" is the Development persona's section (a bare "## Development" reads oddly; see the design doc).
ORDER = [("maintainability", "Maintainability"),
         ("development",      "Correctness"),
         ("security",         "Security"),
         ("hardware",         "Hardware"),
         ("documentation",    "Documentation")]

def load(persona):
    p = os.path.join(fragdir, persona + ".json")
    if not os.path.exists(p):
        return []
    try:
        data = json.load(open(p))
    except Exception as e:
        sys.stderr.write(f"assemble: skipping unparseable {p}: {e}\n")
        return []
    return data if isinstance(data, list) else []

L = ["---",
     f"date: {meta.get('date','')}",
     f"mode: {meta.get('mode','diff')}"]
if meta.get("base"):
    L.append(f"base: {meta['base']}")
if meta.get("files"):
    L.append(f"files: {meta['files']}")
L += [f"head: {meta.get('head','')}",
      f"branch: {meta.get('branch','')}"]
if meta.get("title"):
    # json.dumps yields a valid YAML double-quoted scalar (escapes embedded quotes).
    L.append("title: " + json.dumps(meta["title"]))
L += ["---", "", "# Summary", "", "<!-- SUMMARY -->", ""]

counts = {}
for persona, title in ORDER:
    # Dedup is per-persona: identical comments from two different personas are both kept,
    # in their own sections (design §4 / ADR-0003).
    seen = set()
    uniq = []
    for c in load(persona):
        key = (c.get("file"), c.get("line"), c.get("problem"), c.get("fix"))
        if key in seen:
            continue
        seen.add(key)
        uniq.append(c)
    counts[persona] = len(uniq)
    if not uniq:
        continue
    # Sort by file, then line, then a stable text key
    # so co-located comments have a deterministic order regardless of pass output order.
    uniq.sort(key=lambda c: (str(c.get("file", "")),
                             int(c.get("line", 0) or 0),
                             str(c.get("grounding", "")),
                             str(c.get("problem", ""))))
    L += [f"## {title}", ""]
    for c in uniq:
        loc = "`%s`" % c.get("file", "?")
        if c.get("line"):
            loc += " line %s" % c["line"]
        L += [f"### {loc}", ""]
        diff = c.get("diff")
        if diff:
            L.append("> ```diff")
            L += ["> " + dl for dl in str(diff).splitlines()]
            L += ["> ```", ""]
        L.append("`%s` (%s): %s" % (c.get("grounding", "bug"),
                                    c.get("severity", "major"),
                                    (c.get("problem", "") or "").strip()))
        L += ["", "**Fix.** %s" % (c.get("fix", "") or "").strip(), ""]

open(out, "w").write("\n".join(L).rstrip() + "\n")
sys.stderr.write("assemble: " + ", ".join(f"{p}={counts[p]}" for p, _ in ORDER) + "\n")
PY
