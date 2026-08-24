#!/usr/bin/env bats
# tests/unit/generated-block-placement.bats
#
# scripts/lint-issue.sh exempts generated metadata from the 400-word authored
# budget, but the exemption is line-bounded: only lines between a family's
# begin and end markers are dropped before counting.
#
# A skill template that emits its heading ABOVE the opening marker therefore
# leaks that heading into the authored count. Phase 3 issues routinely land at
# 380-399 words, so the leak is enough to flag an entire batch BODY_TOO_LONG
# the moment Phase 3.5 patches it.
#
# This test pins the invariant at the source: every generated heading in every
# skill template must sit inside its own marker pair.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "skills: every generated heading sits inside its marker pair" {
    run python3 - "$REPO_ROOT" <<'PY'
import pathlib, re, sys

FAMILIES = (
    ("## Model fit",        "autospec-classify"),
    ("## Quality lint",     "autospec-quality"),
    ("## Shared contracts", "autospec-shared-contracts"),
)
LOOKAHEAD = 40

root = pathlib.Path(sys.argv[1])
violations = []
checked = 0

for path in sorted((root / "skills").rglob("*.md")):
    lines = path.read_text().split("\n")
    for heading, marker in FAMILIES:
        begin, end = f"<!-- {marker}:begin -->", f"<!-- {marker}:end -->"
        pattern = re.compile(r"^\s*(?:>\s*)?" + re.escape(heading) + r"\s*$")
        for i, line in enumerate(lines):
            if not pattern.match(line):
                continue
            # Prose that merely NAMES the markers (the templates describe them in
            # a sentence directly above the block) must never be mistaken for the
            # markers themselves, or the check silently passes on any placement.
            def is_marker(line, token):
                stripped = re.sub(r"^\s*(?:>\s*)?", "", line).strip()
                return stripped == token

            close = next((k for k in range(i, min(i + LOOKAHEAD, len(lines)))
                          if is_marker(lines[k], end)), None)
            if close is None:
                continue
            checked += 1
            # The opening marker must be the immediately preceding non-blank line.
            prev = next((k for k in range(i - 1, -1, -1)
                         if lines[k].strip() != ""), None)
            open_ = prev if prev is not None and is_marker(lines[prev], begin) else None
            if open_ is None:
                rel = path.relative_to(root)
                violations.append(
                    f"{rel}:{i + 1}: '{heading}' is above '{begin}'; "
                    f"move the marker above the heading so the block is exempt"
                )

if violations:
    print("\n".join(violations))
    sys.exit(1)
if checked == 0:
    print("no generated-block templates found; the test is not exercising anything")
    sys.exit(1)
print(f"ok: {checked} generated headings, all inside their markers")
PY
    [ "$status" -eq 0 ]
}

@test "emitters outside skills/ wrap generated blocks in their markers" {
    # scripts/autospec-explore.sh shipped a '## Model fit' block with no markers
    # at all, so every issue it filed charged the block to the authored budget.
    # The skills-only glob above could not see it. This covers programmatic
    # emitters, where the heading is a string literal rather than a template.
    run python3 - "$REPO_ROOT" <<'PY'
import pathlib, re, sys

FAMILIES = (
    ("## Model fit",        "autospec-classify"),
    ("## Quality lint",     "autospec-quality"),
    ("## Shared contracts", "autospec-shared-contracts"),
)

root = pathlib.Path(sys.argv[1])
violations = []
checked = 0

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
        for heading, marker in FAMILIES:
            # Match the heading only where it is emitted as a literal line.
            if not re.search(r'(^|["\'])' + re.escape(heading) + r'(["\']|$)', text, re.M):
                continue
            checked += 1
            rel = path.relative_to(root)
            for side in ("begin", "end"):
                if f"<!-- {marker}:{side} -->" not in text:
                    violations.append(
                        f"{rel}: emits '{heading}' but never writes "
                        f"'<!-- {marker}:{side} -->'; the block is charged to the "
                        f"authored word budget"
                    )

if violations:
    print("\n".join(violations))
    sys.exit(1)
if checked == 0:
    print("no programmatic emitters found; the test is not exercising anything")
    sys.exit(1)
print(f"ok: {checked} emitted blocks, all marker-wrapped")
PY
    [ "$status" -eq 0 ]
}
