#!/usr/bin/env python3
"""Parse an aster-code-review review file into GitHub review pieces.

Reads two environment variables and prints JSON to stdout:
  REVIEW_FILE  path to the review file (required)
  DIFF         the PR's unified diff (optional); comments whose line is not on the
               RIGHT side of the diff are moved to "dropped" (GitHub silently
               discards inline comments off the diff). Empty DIFF drops nothing.

Output: {"summary": <str>, "comments": [<postable>], "dropped": [<off-diff>]}
where each comment is {"path", "body", and either "line"/"side"
(+ "start_line"/"start_side" for a range) or "subject_type": "file"}.

Kept model-free and importable so tests/test-post-reviews.sh can exercise it
directly, without gh or the network.
"""
import re, json, os, sys


def parse(text, diff):
    # strip YAML frontmatter (first --- ... --- pair)
    lines = text.split("\n")
    if lines and lines[0].strip() == "---":
        for i in range(1, len(lines)):
            if lines[i].strip() == "---":
                text = "\n".join(lines[i + 1:]); break

    # review body = `# Summary` .. first `## ` (persona); keep consolidated-fix blockquotes
    summary = ""
    m = re.search(r'^# Summary\s*$', text, re.M)
    if m:
        rest = text[m.end():]
        m2 = re.search(r'^## ', rest, re.M)
        summary = (rest[:m2.start()] if m2 else rest).strip()
        summary = re.sub(r'\n*-{3,}\s*$', '', summary).strip()   # drop a trailing --- rule

    # comments: each `### `path` line N` (or `lines N-M`, or bare path = file-level)
    heading = re.compile(r'^###\s+`([^`]+)`(?:\s+lines?\s+(\d+)(?:-(\d+))?)?\s*$', re.M)
    hits = list(heading.finditer(text))
    comments = []
    for i, mm in enumerate(hits):
        path, ls, le = mm.group(1), mm.group(2), mm.group(3)
        seg = text[mm.end(): hits[i + 1].start() if i + 1 < len(hits) else len(text)]
        kept = []
        for ln in seg.split("\n"):
            s = ln.strip()
            if s.startswith("##"):       # reached the next persona section
                break
            if s.startswith(">"):         # the quoted diff block — GitHub shows the diff already
                continue
            kept.append(ln)
        body = "\n".join(kept).strip()
        if not body:
            continue
        c = {"path": path, "body": body}
        if ls:
            c["line"] = int(le) if le else int(ls); c["side"] = "RIGHT"
            if le and le != ls:
                c["start_line"] = int(ls); c["start_side"] = "RIGHT"
        else:
            c["subject_type"] = "file"
        comments.append(c)

    # RIGHT-side line numbers present in the PR diff (added + context); GitHub drops the rest
    postable, cur, newno = {}, None, 0
    for ln in (diff or "").split("\n"):
        if ln.startswith("+++ b/"):
            cur = ln[6:]; postable.setdefault(cur, set())
        elif ln.startswith("+++ "):
            cur = None
        elif ln.startswith("@@"):
            m = re.search(r'\+(\d+)', ln); newno = int(m.group(1)) if m else 0
        elif cur is not None:
            if ln.startswith("+"):
                postable[cur].add(newno); newno += 1
            elif ln.startswith("-"):
                pass
            elif not ln.startswith("\\"):    # context line (incl. blank); ignore "\ No newline"
                postable[cur].add(newno); newno += 1

    # with no diff, nothing is dropped (postable empty -> treat all as kept)
    have_diff = bool(postable)
    kept_c, dropped = [], []
    for c in comments:
        if not have_diff:
            kept_c.append(c)
        elif c.get("subject_type") == "file":
            (kept_c if c["path"] in postable else dropped).append(c)
        elif c["path"] in postable and c["line"] in postable[c["path"]]:
            kept_c.append(c)
        else:
            dropped.append(c)
    return {"summary": summary, "comments": kept_c, "dropped": dropped}


if __name__ == "__main__":
    rf = os.environ.get("REVIEW_FILE")
    if not rf:
        sys.stderr.write("parse-review.py: REVIEW_FILE is required\n"); sys.exit(2)
    print(json.dumps(parse(open(rf).read(), os.environ.get("DIFF", ""))))
