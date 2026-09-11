#!/usr/bin/env python3

# SPDX-License-Identifier: MPL-2.0

"""Validate and query Asterinas's persona-keyed coding guidelines.

Reads command-line arguments and prints query results to stdout:
  catalog <persona>                    print a persona's short-name/gist index
  show --expect-digest <digest>
       <persona> <short-name> [...]    print exact authored rule sections

The optional ACR_GUIDELINE_ROOT environment variable overrides guideline root
resolution. Otherwise, a bundled benchmark snapshot is preferred when present,
then the repository root is used.

Output: command-specific text on stdout. ``catalog`` and ``show`` include the
validated corpus digest so a reviewer cannot mix an index with rule sections
from another guideline revision. Errors are printed to stderr and exit with
status 2.

Kept model-free and deterministic so progressive disclosure exposes the full
compact catalog first, then only the exact rule sections selected by a reviewer.
Every command validates the complete corpus before producing output, preventing
the catalog from silently drifting away from its detail pages.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path


PERSONAS = (
    "maintainability",
    "development",
    "security",
    "hardware",
    "documentation",
)
GUIDELINE_REL = Path("book/src/to-contribute/coding-guidelines")
# These expressions define the authored guideline schema.  They intentionally
# accept less than general Markdown so an index entry always maps to one exact
# H3 rule section and one stable short-name.
SHORT_NAME = r"[a-z0-9][a-z0-9-]*"
INDEX_ITEM_RE = re.compile(
    rf"^\s*-\s+\[`(?P<id>{SHORT_NAME})`\]"
    rf"\((?P<path>[^)#]+)#(?P<anchor>{SHORT_NAME})\):\s+(?P<gist>.+?)\s*$"
)
RULE_HEADING_RE = re.compile(
    rf"^(?P<marks>#{{1,6}})\s+.+\(`(?P<id>{SHORT_NAME})`\)\s+"
    rf"\{{#(?P<anchor>{SHORT_NAME})\}}\s*$"
)
HEADING_RE = re.compile(r"^(?P<marks>#{1,6})\s+")
FENCE_RE = re.compile(r"^\s{0,3}(?P<fence>`{3,}|~{3,})")


class GuidelineError(Exception):
    """A malformed corpus or invalid query."""


@dataclass(frozen=True)
class IndexEntry:
    """Lightweight rule metadata shown in the initial persona catalog."""

    short_name: str
    gist: str
    source: Path
    anchor: str


@dataclass(frozen=True)
class Rule:
    """An exact rule section withheld until ``show`` requests it."""

    short_name: str
    anchor: str
    source: Path
    chunk: str


@dataclass(frozen=True)
class PersonaCorpus:
    """A validated persona index, its detail chunks, and their shared digest."""

    persona: str
    root: Path
    readme: Path
    readme_text: str
    entries: tuple[IndexEntry, ...]
    rules: dict[str, Rule]
    markdown_files: tuple[Path, ...]
    digest: str

    @property
    def catalog_bytes(self) -> int:
        return len(self.readme.read_bytes())

def package_root() -> Path:
    return Path(__file__).resolve().parents[1]


def validate_root(root: Path, source: str) -> Path:
    root = root.resolve()
    guideline_dir = root / GUIDELINE_REL
    if not guideline_dir.is_dir():
        raise GuidelineError(
            f"{source} guideline root does not contain {GUIDELINE_REL}: {root}"
        )
    return root


def resolve_root() -> Path:
    explicit = os.environ.get("ACR_GUIDELINE_ROOT")
    if explicit:
        return validate_root(Path(explicit), "ACR_GUIDELINE_ROOT")

    # Benchmark overlays bundle a guideline snapshot beside the skill.  Using
    # it for both catalog and show prevents the reviewed worktree from changing
    # the rules halfway through a pass.
    bundled = package_root() / "guideline-root"
    if bundled.exists():
        if not bundled.is_dir():
            raise GuidelineError(f"bundled guideline root is not a directory: {bundled}")
        return validate_root(bundled, "bundled")

    if (package_root() / "guideline-root.required").exists():
        raise GuidelineError(
            "bundled guideline snapshot is required but guideline-root is missing"
        )

    current = Path.cwd().resolve()
    for candidate in (current, *current.parents):
        if (candidate / GUIDELINE_REL).is_dir():
            return validate_root(candidate, "current repository")
    raise GuidelineError(
        f"cannot find {GUIDELINE_REL} from {current}; set ACR_GUIDELINE_ROOT"
    )


def is_fence_close(line: str, fence: str) -> bool:
    indent = len(line) - len(line.lstrip(" "))
    if indent > 3:
        return False
    stripped = line[indent:]
    marker = re.escape(fence[0])
    return re.fullmatch(rf"{marker}{{{len(fence)},}}\s*", stripped) is not None


def structural_headings(lines: list[str]) -> list[tuple[int, int, re.Match[str] | None]]:
    """Return real Markdown headings, ignoring examples inside code fences."""

    headings: list[tuple[int, int, re.Match[str] | None]] = []
    fence: str | None = None
    for index, line in enumerate(lines):
        if fence is not None:
            if is_fence_close(line, fence):
                fence = None
            continue

        fence_match = FENCE_RE.match(line)
        if fence_match:
            fence = fence_match.group("fence")
            continue

        heading_match = HEADING_RE.match(line)
        if heading_match:
            headings.append(
                (index, len(heading_match.group("marks")), RULE_HEADING_RE.match(line))
            )
    return headings


def parse_rules(path: Path, root: Path) -> list[Rule]:
    """Extract each exact H3 rule chunk through its next peer/parent heading."""

    text = path.read_text(encoding="utf-8")
    lines = text.splitlines(keepends=True)
    headings = structural_headings(lines)
    rules: list[Rule] = []

    for heading_index, (line_index, level, rule_match) in enumerate(headings):
        if level == 3 and rule_match is None:
            raise GuidelineError(
                f"malformed H3 guideline heading at {path}:{line_index + 1}"
            )
        if rule_match is None:
            continue
        if level != 3:
            raise GuidelineError(f"rule heading must be H3: {path}:{line_index + 1}")

        short_name = rule_match.group("id")
        anchor = rule_match.group("anchor")
        if short_name != anchor:
            raise GuidelineError(
                f"rule short-name and anchor differ at {path}:{line_index + 1}: "
                f"{short_name} != {anchor}"
            )

        end_index = len(lines)
        for next_line, next_level, _ in headings[heading_index + 1 :]:
            if next_level <= level:
                end_index = next_line
                break

        rules.append(
            Rule(
                short_name=short_name,
                anchor=anchor,
                source=path.relative_to(root),
                chunk="".join(lines[line_index:end_index]),
            )
        )
    return rules


def resolve_index_target(persona_dir: Path, link: str) -> Path:
    # An index is allowed to select detail only from its own persona corpus.
    target = (persona_dir / link).resolve()
    try:
        target.relative_to(persona_dir.resolve())
    except ValueError as error:
        raise GuidelineError(f"guideline link escapes persona directory: {link}") from error
    if not target.is_file():
        raise GuidelineError(f"guideline link target does not exist: {link}")
    return target


def parse_index(readme: Path, persona_dir: Path, root: Path) -> tuple[IndexEntry, ...]:
    """Parse the complete short-name/gist catalog from a persona README."""

    entries: list[IndexEntry] = []
    seen: set[str] = set()
    fence: str | None = None

    for line_number, line in enumerate(readme.read_text(encoding="utf-8").splitlines(), 1):
        if fence is not None:
            if is_fence_close(line, fence):
                fence = None
            continue
        fence_match = FENCE_RE.match(line)
        if fence_match:
            fence = fence_match.group("fence")
            continue

        match = INDEX_ITEM_RE.match(line)
        if match is None:
            if re.match(rf"^\s*-\s+\[`{SHORT_NAME}`\]\(", line):
                raise GuidelineError(
                    f"malformed guideline index item at {readme}:{line_number}"
                )
            continue
        short_name = match.group("id")
        anchor = match.group("anchor")
        if short_name != anchor:
            raise GuidelineError(
                f"index short-name and anchor differ at {readme}:{line_number}: "
                f"{short_name} != {anchor}"
            )
        if short_name in seen:
            raise GuidelineError(f"duplicate index short-name: {short_name}")
        seen.add(short_name)
        source = resolve_index_target(persona_dir, match.group("path"))
        entries.append(
            IndexEntry(
                short_name=short_name,
                gist=match.group("gist"),
                source=source.relative_to(root),
                anchor=anchor,
            )
        )

    if not entries:
        raise GuidelineError(f"persona index has no guideline entries: {readme}")
    return tuple(entries)


def digest_files(paths: tuple[Path, ...], root: Path) -> str:
    # Hash both relative paths and bytes: renaming a source page must invalidate
    # a catalog digest even when its content is unchanged.  NUL separators keep
    # adjacent path/content pairs unambiguous.
    digest = hashlib.sha256()
    for path in paths:
        relative = path.relative_to(root).as_posix().encode()
        digest.update(relative)
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def load_persona(root: Path, persona: str) -> PersonaCorpus:
    """Compile and cross-check one persona's catalog and detail pages."""

    if persona not in PERSONAS:
        raise GuidelineError(
            f"unknown persona: {persona}; valid: {', '.join(PERSONAS)}"
        )

    persona_dir = root / GUIDELINE_REL / f"for-{persona}"
    readme = persona_dir / "README.md"
    if not readme.is_file():
        raise GuidelineError(f"missing persona index: {readme}")

    markdown_files = tuple(sorted(persona_dir.rglob("*.md")))
    entries = parse_index(readme, persona_dir, root)
    rules_by_id: dict[str, Rule] = {}
    rules_by_locus: dict[tuple[Path, str], Rule] = {}

    # Build both lookup keys so the public short-name and the README's concrete
    # path#anchor target can be checked independently.
    for path in markdown_files:
        for rule in parse_rules(path, root):
            if rule.short_name in rules_by_id:
                raise GuidelineError(f"duplicate rule short-name: {rule.short_name}")
            locus = (rule.source, rule.anchor)
            if locus in rules_by_locus:
                raise GuidelineError(
                    f"duplicate rule source and anchor: {rule.source}#{rule.anchor}"
                )
            rules_by_id[rule.short_name] = rule
            rules_by_locus[locus] = rule

    # Enforce both directions of completeness: every catalog entry must resolve
    # to exactly the named rule, and every authored rule must appear in the
    # catalog.  The model can therefore treat the compact catalog as exhaustive.
    indexed_ids: set[str] = set()
    for entry in entries:
        locus = (entry.source, entry.anchor)
        rule = rules_by_locus.get(locus)
        if rule is None:
            raise GuidelineError(
                f"index target is not a rule heading: {entry.source}#{entry.anchor}"
            )
        if rule.short_name != entry.short_name:
            raise GuidelineError(
                f"index target {entry.source}#{entry.anchor} names {rule.short_name}, "
                f"not {entry.short_name}"
            )
        indexed_ids.add(entry.short_name)

    orphaned = sorted(set(rules_by_id) - indexed_ids)
    if orphaned:
        raise GuidelineError(f"orphan rules absent from {readme}: {', '.join(orphaned)}")

    return PersonaCorpus(
        persona=persona,
        root=root,
        readme=readme,
        readme_text=readme.read_text(encoding="utf-8"),
        entries=entries,
        rules=rules_by_id,
        markdown_files=markdown_files,
        digest=digest_files(markdown_files, root),
    )


def validate_corpora(root: Path, personas: list[str]) -> list[PersonaCorpus]:
    corpora = [load_persona(root, persona) for persona in personas]
    # A finding carries a short-name as its grounding.  Global uniqueness keeps
    # that identifier meaningful even when a combined pass sees several personas.
    owners: dict[str, str] = {}
    for corpus in corpora:
        for entry in corpus.entries:
            previous = owners.get(entry.short_name)
            if previous is not None:
                raise GuidelineError(
                    f"rule short-name {entry.short_name} belongs to both "
                    f"{previous} and {corpus.persona}"
                )
            owners[entry.short_name] = corpus.persona
    return corpora


def command_catalog(corpus: PersonaCorpus) -> None:
    """Emit disclosure level 1: complete rule names/gists, but no detail pages."""

    print(
        f"GUIDELINE_CATALOG persona={corpus.persona} rules={len(corpus.entries)} "
        f"digest={corpus.digest} bytes={corpus.catalog_bytes}"
    )
    print(f"source={corpus.readme.relative_to(corpus.root).as_posix()}")
    print()
    sys.stdout.write(corpus.readme_text)
    if not corpus.readme_text.endswith("\n"):
        print()


def command_show(
    corpus: PersonaCorpus, requested: list[str], expected_digest: str
) -> None:
    """Emit disclosure level 2: exact chunks for explicitly requested rules."""

    # Pin the lookup to the corpus that produced the prompt's catalog.  A stale
    # pass must rebuild its prompt instead of mixing two guideline revisions.
    if expected_digest != corpus.digest:
        raise GuidelineError(
            f"guideline digest mismatch for {corpus.persona}: "
            f"expected {expected_digest}, resolved {corpus.digest}"
        )

    requested_set = set(requested)
    valid_ids = [entry.short_name for entry in corpus.entries]
    unknown = sorted(requested_set - set(valid_ids))
    if unknown:
        raise GuidelineError(
            f"unknown guideline for {corpus.persona}: {', '.join(unknown)}; "
            f"valid: {', '.join(valid_ids)}"
        )

    # Preserve canonical catalog order rather than model-supplied argument order;
    # this makes tool transcripts deterministic and friendlier to prompt caching.
    selected = [entry for entry in corpus.entries if entry.short_name in requested_set]
    chunk_bytes = sum(
        len(corpus.rules[entry.short_name].chunk.encode("utf-8")) for entry in selected
    )
    print(
        f"GUIDELINE_CHUNKS persona={corpus.persona} "
        f"ids={','.join(entry.short_name for entry in selected)} "
        f"digest={corpus.digest} bytes={chunk_bytes}"
    )
    for entry in selected:
        rule = corpus.rules[entry.short_name]
        print()
        print(f"--- rule: {entry.short_name} ---")
        print(f"source: {entry.source.as_posix()}#{entry.anchor}")
        print()
        sys.stdout.write(rule.chunk)
        if not rule.chunk.endswith("\n"):
            print()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate and query persona-keyed Asterinas coding guidelines."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    catalog = subparsers.add_parser("catalog", help="print one persona's gist index")
    catalog.add_argument("persona", choices=PERSONAS)

    show = subparsers.add_parser("show", help="print exact guideline rule sections")
    show.add_argument("--expect-digest", required=True)
    show.add_argument("persona", choices=PERSONAS)
    show.add_argument("short_names", nargs="+")

    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        root = resolve_root()
        # Validate all personas on every query, even when only one is printed.
        # Cross-persona duplicate IDs would otherwise depend on which command ran.
        all_corpora = validate_corpora(root, list(PERSONAS))
        corpora_by_persona = {corpus.persona: corpus for corpus in all_corpora}
        corpus = corpora_by_persona[args.persona]
        if args.command == "catalog":
            command_catalog(corpus)
        elif args.command == "show":
            command_show(corpus, args.short_names, args.expect_digest)
    except (GuidelineError, OSError, UnicodeError) as error:
        print(f"print_guideline.py: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
