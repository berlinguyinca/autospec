#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SKILL="$REPO_ROOT/skills/autospec-run/SKILL.md"
  INSTALLER="$REPO_ROOT/skills/autospec-run/install.sh"
  DEFINE_INSTALLER="$REPO_ROOT/skills/autospec-define/install.sh"
}

@test "handoff installers ship executable runtime and all versioned schemas" {
  run sh "$INSTALLER" --harness codex --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-handoff.py"* ]]
  [[ "$output" == *"autospec-pi-bridge-dispatch.py"* ]]
  [[ "$output" == *"autospec-agent-handoff-result-v1.schema.json"* ]]
  [[ "$output" == *"autospec-implementation-handoff-v1.schema.json"* ]]
  [[ "$output" == *"autospec-review-handoff-v1.schema.json"* ]]
  [[ "$output" == *"autospec-spec-v1.schema.json"* ]]

  run sh "$DEFINE_INSTALLER" --harness codex --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-handoff.py"* ]]
  [[ "$output" == *"autospec-pi-bridge-dispatch.py"* ]]
  [[ "$output" == *"autospec-spec-v1.schema.json"* ]]
  [[ "$output" == *"autospec-agent-handoff-result-v1.schema.json"* ]]
}

@test "define and run prompt trios expose the same handoff protocol" {
  run bash "$REPO_ROOT/scripts/derive-trio.sh" skills/autospec-define --check
  [ "$status" -eq 0 ]
  run bash "$REPO_ROOT/scripts/derive-trio.sh" skills/autospec-run --check
  [ "$status" -eq 0 ]

  run python3 - "$REPO_ROOT/skills/autospec-define/SKILL.md" "$SKILL" <<'PY'
from pathlib import Path
import sys

define, run = (Path(path).read_text() for path in sys.argv[1:])
for token in ("AUTOSPEC_PI_HANDOFF_CONFIG", "autospec-pi-bridge-dispatch.py", "reconcile-spec"):
    assert token in define
for token in ("autospec-implementation-handoff-v1", "autospec-review-handoff-v1", "accept-result"):
    assert token in run
PY
  [ "$status" -eq 0 ]
}

@test "implementer routing is opt-in and retains the legacy selector fallback" {
  run python3 - "$SKILL" <<'PY'
from pathlib import Path
import sys

body = Path(sys.argv[1]).read_text()
route = body.index("AUTOSPEC_ROUTING_CONFIG")
resolve = body.index("autospec-route.py", route)
fallback = body.index("select-model-profile.sh", resolve)
assert "--kind execution" in body[resolve:fallback]
assert "autospec-pi-dispatch.py" in body[resolve:fallback]
assert "legacy selector exactly" in body[resolve:fallback].lower()
PY
  [ "$status" -eq 0 ]
}

@test "unified execution routing does not alter reviewer tier policy" {
  run python3 - "$SKILL" <<'PY'
from pathlib import Path
import sys

body = Path(sys.argv[1]).read_text()
review = body.index("AUTOSPEC_REVIEWER_TIER")
window = body[review:review + 1800]
assert "autospec-route.py" not in window
assert "autospec-pi-dispatch.py" not in window
assert "TIER_B" in window
PY
  [ "$status" -eq 0 ]
}

@test "autospec-run installer ships unified routing runtime and schemas" {
  for name in autospec-route.py autospec_route_lib.py autospec-pi-dispatch.py; do
    run grep -F "$name" "$INSTALLER"
    [ "$status" -eq 0 ]
  done
  for name in autospec-routing-v1.schema.json inferweave-capabilities-v1.schema.json autospec-dispatch-envelope-v1.schema.json; do
    run grep -F "$name" "$INSTALLER"
    [ "$status" -eq 0 ]
  done
  run grep -F 'AUTOSPEC_SCHEMAS_DIR' "$INSTALLER"
  [ "$status" -eq 0 ]
}

@test "autospec-run prompt trio remains derived from the canonical skill" {
  run bash "$REPO_ROOT/scripts/derive-trio.sh" skills/autospec-run --check
  [ "$status" -eq 0 ]
}
