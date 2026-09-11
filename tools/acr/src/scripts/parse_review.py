#!/usr/bin/env python3
"""Parse an aster-code-review review file into GitHub review pieces.

Reads environment variables and prints JSON to stdout:
  REVIEW_FILE  path to the review file (required)
  DIFF_FILE    path to a file holding the PR's unified diff (preferred). A large diff
               passed inline via DIFF overflows the exec arg list, so callers write it
               to a file and pass the path here.
  DIFF         the PR's unified diff inline (optional fallback, used by the tests).
               Comments whose line is not on the RIGHT side of the diff are moved to
               "dropped" (GitHub silently discards inline comments off the diff).
               An empty/absent diff drops nothing.

Output: {"summary": <str>, "comments": [<postable>], "dropped": [<off-diff>],
         "dropped_section": <str>}
where each comment is {"path", "body", and either "line"/"side"
(+ "start_line"/"start_side" for a range) or "subject_type": "file"}.
"dropped_section" is a ready-to-append Markdown rendering of "dropped" (empty when
nothing was dropped): the poster puts it below the summary so off-diff findings,
which GitHub cannot inline, are surfaced instead of silently lost.

Kept model-free and importable so tests/test_post_reviews.sh can exercise it
directly, without gh or the network.
"""
import re, json, os, sys


def render_dropped(dropped):
    """Render off-diff findings as a Markdown section for the review body.

    GitHub silently discards an inline comment whose line is not on the diff, so
    these would be lost. We surface them below the summary instead, hedged: a
    dropped finding is usually on a line this PR did not change (most likely a
    pre-existing issue) or is about a commit message. Returns "" when nothing was
    dropped. Location matches the inline form: `path` line N, or the bare `path`
    (a filename, or a "commit <sha> message" locus) when there is no line.
    """
    if not dropped:
        return ""
    out = ["---", "",
           "### Findings not attachable to this PR's diff", "",
           "> [!NOTE]",
           "> These couldn't be pinned to a changed line",
           "> — usually they sit on lines this PR didn't change (likely pre-existing issues),",
           "> or concern a commit message.",
           "> Listed here so they aren't lost.", ""]
    for c in dropped:
        if not c.get("line"):
            loc = "`%s`" % c["path"]                                  # file-level / commit message
        elif c.get("start_line") and c["start_line"] != c["line"]:
            loc = "`%s` lines %s-%s" % (c["path"], c["start_line"], c["line"])
        else:
            loc = "`%s` line %s" % (c["path"], c["line"])
        out += ["**%s** — %s" % (loc, c["body"]), ""]
    return "\n".join(out).rstrip()


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

    # RIGHT-side line numbers present in the PR diff (added + context); GitHub drops the rest.
    # File boundaries are keyed off `diff --git`, and lines are counted ONLY inside a hunk, so
    # that (a) an added line whose own text starts with `+++ `/`--- ` is treated as content, not
    # a file header, and (b) the inter-file `diff --git`/`index` lines are not miscounted as
    # context for the previous file.
    postable, cur, newno, in_hunk = {}, None, 0, False
    for ln in (diff or "").split("\n"):
        if ln.startswith("diff --git "):
            cur, in_hunk = None, False       # leaving the previous file; next `+++ b/` sets cur
        elif ln.startswith("@@"):
            m = re.search(r'\+(\d+)', ln); newno = int(m.group(1)) if m else 0
            in_hunk = True
        elif not in_hunk and ln.startswith("+++ b/"):
            cur = ln[6:]; postable.setdefault(cur, set())
        elif not in_hunk and ln.startswith("+++ "):
            cur = None                       # e.g. `+++ /dev/null` (deleted file: no RIGHT side)
        elif in_hunk and cur is not None:
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
    return {"summary": summary, "comments": kept_c, "dropped": dropped,
            "dropped_section": render_dropped(dropped)}


if __name__ == "__main__":
    rf = os.environ.get("REVIEW_FILE")
    if not rf:
        sys.stderr.write("parse_review.py: REVIEW_FILE is required\n"); sys.exit(2)
    # Prefer DIFF_FILE (a path)
    # — a large PR diff overflows the exec arg list if passed inline via DIFF.
    # DIFF (inline) remains for the model-free tests.
    df = os.environ.get("DIFF_FILE")
    diff = open(df).read() if df else os.environ.get("DIFF", "")
    print(json.dumps(parse(open(rf).read(), diff)))
