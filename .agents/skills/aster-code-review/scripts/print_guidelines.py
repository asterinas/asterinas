#!/usr/bin/env python3

# SPDX-License-Identifier: MPL-2.0

"""Print Asterinas coding-guideline catalogs or selected guidelines.

The common interface is deliberately small:

  print_guidelines.py <persona> --catalog
  print_guidelines.py <persona> [<short-name> ...]

The first form prints the persona's compact short-name/gist catalog. The
second prints the exact authored sections for the requested short-names in
catalog order; with no short-name, it prints every indexed guideline for that
persona.

The optional ACR_GUIDELINE_ROOT environment variable overrides guideline-root
resolution. Otherwise, a bundled benchmark snapshot is used,
then the repository root is used. Errors are printed to stderr and exit with
status 2.
"""

from __future__ import annotations

import argparse
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
SHORT_NAME = r"[a-z0-9][a-z0-9-]*"
INDEX_ITEM_RE = re.compile(
    rf"^\s*-\s+\[`(?P<id>{SHORT_NAME})`\]"
    rf"\((?P<path>[^)#]+)#(?P<anchor>{SHORT_NAME})\):\s+(?P<gist>.+?)\s*$"
)
RULE_HEADING_RE = re.compile(
    rf"^(?P<marks>#{{1,6}})\s+.+\(`(?P<id>{SHORT_NAME})`\)\s+"
    rf"{{#(?P<anchor>{SHORT_NAME})}}\s*$"
)
HEADING_RE = re.compile(r"^(?P<marks>#{1,6})\s+")
FENCE_RE = re.compile(r"^\s{0,3}(?P<fence>`{3,}|~{3,})")


class GuidelineError(Exception):
    """An invalid guideline layout or request."""


@dataclass(frozen=True)
class IndexEntry:
    """Lightweight guideline metadata from a persona index."""

    short_name: str
    gist: str
    source: Path
    anchor: str


@dataclass(frozen=True)
class Rule:
    """An exact authored guideline section."""

    short_name: str
    anchor: str
    source: Path
    chunk: str


@dataclass(frozen=True)
class PersonaCatalog:
    """One persona's compact index and root metadata."""

    persona: str
    root: Path
    readme: Path
    readme_text: str
    entries: tuple[IndexEntry, ...]


def skill_dir() -> Path:
    return Path(__file__).resolve().parent.parent


def check_root(root: Path, source: str) -> Path:
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
        return check_root(Path(explicit), "ACR_GUIDELINE_ROOT")

    # Benchmark overlays keep current guidelines beside the skill so a pass in
    # a historical worktree cannot accidentally read that worktree's old book.
    bundled = skill_dir() / "guideline-root"
    if bundled.exists():
        if not bundled.is_dir():
            raise GuidelineError(f"bundled guideline root is not a directory: {bundled}")
        return check_root(bundled, "bundled")

    if (skill_dir() / "guideline-root.required").exists():
        raise GuidelineError(
            "bundled guideline snapshot is required but guideline-root is missing"
        )

    return check_root(skill_dir().parents[2], "repository")


def is_fence_close(line: str, fence: str) -> bool:
    indent = len(line) - len(line.lstrip(" "))
    if indent > 3:
        return False
    stripped = line[indent:]
    marker = re.escape(fence[0])
    return re.fullmatch(rf"{marker}{{{len(fence)},}}\s*", stripped) is not None


def structural_headings(lines: list[str]) -> list[tuple[int, int, re.Match[str] | None]]:
    """Return Markdown headings outside fenced code blocks."""

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
    """Extract each H3 guideline section through its next peer/parent heading."""

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
    target = (persona_dir / link).resolve()
    try:
        target.relative_to(persona_dir.resolve())
    except ValueError as error:
        raise GuidelineError(f"guideline link escapes persona directory: {link}") from error
    if not target.is_file():
        raise GuidelineError(f"guideline link target does not exist: {link}")
    return target


def parse_index(readme: Path, persona_dir: Path, root: Path) -> tuple[IndexEntry, ...]:
    """Parse the short-name/gist entries in one persona README."""

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


def load_catalog(root: Path, persona: str) -> PersonaCatalog:
    if persona not in PERSONAS:
        raise GuidelineError(
            f"unknown persona: {persona}; valid: {', '.join(PERSONAS)}"
        )

    persona_dir = root / GUIDELINE_REL / f"for-{persona}"
    readme = persona_dir / "README.md"
    if not readme.is_file():
        raise GuidelineError(f"missing persona index: {readme}")
    readme_text = readme.read_text(encoding="utf-8")
    return PersonaCatalog(
        persona=persona,
        root=root,
        readme=readme,
        readme_text=readme_text,
        entries=parse_index(readme, persona_dir, root),
    )


def select_rules(catalog: PersonaCatalog, requested: list[str]) -> list[Rule]:
    """Read only the pages needed for the requested short-names."""

    valid_ids = [entry.short_name for entry in catalog.entries]
    requested_set = set(requested) if requested else set(valid_ids)
    unknown = sorted(requested_set - set(valid_ids))
    if unknown:
        raise GuidelineError(
            f"unknown guideline for {catalog.persona}: {', '.join(unknown)}; "
            f"valid: {', '.join(valid_ids)}"
        )

    selected_entries = [
        entry for entry in catalog.entries if entry.short_name in requested_set
    ]
    selected_sources = {entry.source for entry in selected_entries}
    rules_by_locus: dict[tuple[Path, str], Rule] = {}
    for source in sorted(selected_sources):
        for rule in parse_rules(catalog.root / source, catalog.root):
            locus = (rule.source, rule.anchor)
            if locus in rules_by_locus:
                raise GuidelineError(
                    f"duplicate rule source and anchor: {rule.source}#{rule.anchor}"
                )
            rules_by_locus[locus] = rule

    selected_rules: list[Rule] = []
    for entry in selected_entries:
        rule = rules_by_locus.get((entry.source, entry.anchor))
        if rule is None:
            raise GuidelineError(
                f"index target is not a rule heading: {entry.source}#{entry.anchor}"
            )
        if rule.short_name != entry.short_name:
            raise GuidelineError(
                f"index target {entry.source}#{entry.anchor} names {rule.short_name}, "
                f"not {entry.short_name}"
            )
        selected_rules.append(rule)
    return selected_rules


def print_catalog(catalog: PersonaCatalog) -> None:
    print(
        f"GUIDELINE_CATALOG persona={catalog.persona} "
        f"rules={len(catalog.entries)}"
    )
    print(f"source={catalog.readme.relative_to(catalog.root).as_posix()}")
    print()
    sys.stdout.write(catalog.readme_text)
    if not catalog.readme_text.endswith("\n"):
        print()


def print_rules(catalog: PersonaCatalog, requested: list[str]) -> None:
    rules = select_rules(catalog, requested)
    print(
        f"GUIDELINES persona={catalog.persona} "
        f"short-names={','.join(rule.short_name for rule in rules)}"
    )
    for rule in rules:
        print()
        print(f"--- guideline: {rule.short_name} ---")
        print(f"source: {rule.source.as_posix()}#{rule.anchor}")
        print()
        sys.stdout.write(rule.chunk)
        if not rule.chunk.endswith("\n"):
            print()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Print coding guidelines for one Asterinas review persona."
    )
    parser.add_argument("persona", choices=PERSONAS)
    parser.add_argument(
        "short_names",
        metavar="short-name",
        nargs="*",
        help="guidelines to print; omit to print every indexed guideline",
    )
    parser.add_argument(
        "--catalog",
        action="store_true",
        help="print the persona's short-name/gist catalog",
    )
    args = parser.parse_args()
    if args.catalog and args.short_names:
        parser.error("--catalog does not accept short-names")
    return args


def main() -> int:
    args = parse_args()
    try:
        catalog = load_catalog(resolve_root(), args.persona)
        if args.catalog:
            print_catalog(catalog)
        else:
            print_rules(catalog, args.short_names)
    except (GuidelineError, OSError, UnicodeError) as error:
        print(f"print_guidelines.py: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
