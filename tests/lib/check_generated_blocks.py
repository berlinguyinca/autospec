#!/usr/bin/env python3
"""Assert generated metadata blocks are enclosed by their markers.

scripts/lint-issue.sh exempts generated blocks from the 400-word authored budget,
but the exemption is line-bounded: only lines between a family's begin and end
markers are dropped. A block whose heading sits outside the markers leaks that
heading into the count, and Phase 3 issues land close enough to the cap that the
leak alone flags them.

Two surfaces are checked:

  templates  skills/**/*.md  - the heading must sit between the markers.
  emitters   scripts, templates, prompts - non-markdown files that WRITE a block.
             Source order must place the begin marker before the heading and the
             end marker after it. Presence alone is not enough: the original bug
             was a marker in the wrong place, not a missing one.

Usage: check_generated_blocks.py <repo-root> [templates|emitters]
"""
import pathlib
import re
import sys

FAMILIES = (
    ("## Model fit", "autospec-classify"),
    ("## Quality lint", "autospec-quality"),
    ("## Shared contracts", "autospec-shared-contracts"),
)
LOOKAHEAD = 40


def _strip_prefix(line):
    return re.sub(r"^\s*(?:>\s*)?", "", line).strip()


def check_templates(root):
    violations, checked = [], 0
    for path in sorted((root / "skills").rglob("*.md")):
        lines = path.read_text().split("\n")
        for heading, marker in FAMILIES:
            begin, end = f"<!-- {marker}:begin -->", f"<!-- {marker}:end -->"
            pattern = re.compile(r"^\s*(?:>\s*)?" + re.escape(heading) + r"\s*$")
            for i, line in enumerate(lines):
                if not pattern.match(line):
                    continue
                # Prose that merely NAMES the markers must never be mistaken for
                # the markers themselves, or the check passes on any placement.
                close = next(
                    (k for k in range(i, min(i + LOOKAHEAD, len(lines)))
                     if _strip_prefix(lines[k]) == end),
                    None,
                )
                if close is None:
                    continue
                checked += 1
                prev = next(
                    (k for k in range(i - 1, -1, -1) if lines[k].strip() != ""),
                    None,
                )
                if prev is None or _strip_prefix(lines[prev]) != begin:
                    violations.append(
                        f"{path.relative_to(root)}:{i + 1}: '{heading}' is not "
                        f"immediately preceded by '{begin}'; move the marker above "
                        f"the heading so the block is exempt"
                    )
    return violations, checked


def _emitted(text, needle):
    """Line numbers where `needle` is written as content, not described in a comment.

    A markdown heading and a shell comment both begin with '#', so a line is
    treated as a comment only when it is not the needle standing alone -- otherwise
    the filter would discard the here-doc line that emits '## Model fit'.
    """
    return [
        n
        for n, line in enumerate(text.split("\n"), 1)
        if needle in line
        and (line.strip() == needle or not line.lstrip().startswith("#"))
    ]


def check_emitters(root):
    violations, checked = [], 0
    for sub in ("scripts", "templates", "prompts"):
        base = root / sub
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*")):
            if not path.is_file() or path.suffix in {".md", ".json", ".yml", ".yaml"}:
                continue
            try:
                text = path.read_text()
            except (UnicodeDecodeError, OSError):
                continue
            lines = text.split("\n")
            for heading, marker in FAMILIES:
                # An emitted heading is written as content: a bare here-doc line,
                # a printf argument ending in an escaped newline, or a quoted list
                # element. Usage prose that quotes the heading mid-sentence is not.
                heads = [
                    n for n in _emitted(text, heading)
                    if re.match(r"^\s*" + re.escape(heading) + r"\s*$", lines[n - 1])
                    or re.search(
                        re.escape(heading) + r"(?:\\n|[\"']\s*,?\s*$)", lines[n - 1]
                    )
                ]
                if not heads:
                    continue
                checked += 1
                rel = path.relative_to(root)
                begins = _emitted(text, f"<!-- {marker}:begin -->")
                ends = _emitted(text, f"<!-- {marker}:end -->")
                if not begins or not ends:
                    violations.append(
                        f"{rel}: emits '{heading}' but does not write both "
                        f"'<!-- {marker}:begin -->' and its ':end'; the block is "
                        f"charged to the authored word budget"
                    )
                    continue
                head = min(heads)
                if not any(b < head for b in begins):
                    violations.append(
                        f"{rel}:{head}: '{heading}' is emitted before "
                        f"'<!-- {marker}:begin -->' (at {begins}); the heading falls "
                        f"outside the exempt region"
                    )
                if not any(e > head for e in ends):
                    violations.append(
                        f"{rel}:{head}: '<!-- {marker}:end -->' is emitted before "
                        f"the heading; the block is not enclosed"
                    )
    return violations, checked


def main():
    if len(sys.argv) < 2:
        print(__doc__.strip().splitlines()[-1])
        return 2
    root = pathlib.Path(sys.argv[1])
    mode = sys.argv[2] if len(sys.argv) > 2 else "templates"
    check = {"templates": check_templates, "emitters": check_emitters}.get(mode)
    if check is None:
        print(f"unknown mode: {mode}")
        return 2
    violations, checked = check(root)
    if violations:
        print("\n".join(violations))
        return 1
    if checked == 0:
        print(f"no {mode} found; the check is not exercising anything")
        return 1
    print(f"ok: {checked} {mode} blocks, all correctly enclosed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
