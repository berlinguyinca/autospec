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
            # Only judge headings that belong to a template block, i.e. one whose
            # closing marker follows within the lookahead window.
            close = next((k for k in range(i, min(i + LOOKAHEAD, len(lines)))
                          if end in lines[k]), None)
            if close is None:
                continue
            checked += 1
            open_ = next((k for k in range(i, max(i - LOOKAHEAD, -1), -1)
                          if begin in lines[k]), None)
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
