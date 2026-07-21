#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/skills/autospec-ui-audit/scripts/route-inventory.mjs"
  FIXTURES="$REPO_ROOT/tests/fixtures/ui-audit-route-inventory"
  OUTPUT="$BATS_TEST_TMPDIR/output"
}

@test "React Router fixture reconciles nested lazy and duplicate discoveries once" {
  run node "$SCRIPT" --repo "$FIXTURES/react-router" --output-dir "$OUTPUT"

  [ "$status" -eq 0 ]
  [ -f "$OUTPUT/route-inventory.json" ]
  [ -f "$OUTPUT/route-inventory.md" ]

  node - "$OUTPUT/route-inventory.json" <<'NODE'
const fs = require("node:fs");
const inventory = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
if (inventory.framework !== "react-router") throw new Error("wrong adapter");
if (inventory.routes.length !== 5) throw new Error(`expected 5 routes, got ${inventory.routes.length}`);
const paths = inventory.routes.map((route) => route.path);
if (new Set(paths).size !== paths.length) throw new Error("final routes are not unique");
const project = inventory.routes.find((route) => route.path === "/projects/:id");
if (!project || project.status !== "runtime-eligible" || project.sources.length !== 2) {
  throw new Error("duplicate nested project route was not reconciled exactly once");
}
const settings = inventory.routes.find((route) => route.path === "/settings");
if (!settings?.lazy) throw new Error("lazy route evidence missing");
const catchAll = inventory.routes.find((route) => route.path === "/*");
if (!catchAll || catchAll.status !== "excluded" || !catchAll.reason) {
  throw new Error("catch-all route lacks explicit exclusion reason");
}
const orphan = inventory.mismatches.find((item) => item.path === "/orphaned-nav");
if (!orphan || orphan.kind !== "registry-only" || !orphan.reason) {
  throw new Error("unexplained navigation route was silently dropped");
}
NODE

  grep -q '^# Route inventory' "$OUTPUT/route-inventory.md"
  grep -q '/orphaned-nav' "$OUTPUT/route-inventory.md"
}

@test "cyclic route collection fails closed without artifacts" {
  run node "$SCRIPT" --repo "$FIXTURES/cycle" --output-dir "$OUTPUT"

  [ "$status" -eq 1 ]
  [[ "$output" == *"ROUTE_INVENTORY_CYCLE"* ]]
  [ ! -e "$OUTPUT/route-inventory.json" ]
  [ ! -e "$OUTPUT/route-inventory.md" ]
}
