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
if (inventory.routes.length !== 7) throw new Error(`expected 7 routes, got ${inventory.routes.length}`);
const paths = inventory.routes.map((route) => route.path);
if (new Set(paths).size !== paths.length) throw new Error("final routes are not unique");
if (paths.includes("/retired")) throw new Error("commented JSX was parsed as a route");
if (paths.includes("/deleted-but-built")) throw new Error("generated dist route was parsed");
if (!paths.includes("/strings")) throw new Error("comment markers inside strings corrupted JSX");
for (const path of ["/single-string", "/double-string", "/template-string", "/retired-after-even-backslashes"]) {
  if (paths.includes(path)) throw new Error(`lexical noise was parsed as a route: ${path}`);
}
if (!paths.includes("/after-lexical-noise")) {
  throw new Error("contraction or even-backslash quote handling hid later JSX");
}
const project = inventory.routes.find((route) => route.path === "/projects/:id");
if (!project || project.status !== "runtime-eligible" || project.sources.length !== 2) {
  throw new Error("duplicate nested project route was not reconciled exactly once");
}
const settings = inventory.routes.find((route) => route.path === "/settings");
if (!settings?.lazy) throw new Error("lazy route evidence missing");
if (settings.registries.length !== 2) {
  throw new Error("query/fragment registry URLs did not reconcile to /settings");
}
const catchAll = inventory.routes.find((route) => route.path === "/*");
if (!catchAll || catchAll.status !== "excluded" || !catchAll.reason) {
  throw new Error("catch-all route lacks explicit exclusion reason");
}
const orphan = inventory.mismatches.find((item) => item.path === "/orphaned-nav");
if (!orphan || orphan.kind !== "registry-only" || !orphan.reason) {
  throw new Error("unexplained navigation route was silently dropped");
}
const ghosts = inventory.mismatches.filter((item) => item.path === "/ghost");
if (ghosts.length !== 1 || ghosts[0].sources.length !== 2) {
  throw new Error("duplicate mismatch records were not reconciled with both sources");
}
if (inventory.mismatches.some((item) => /[?#]/.test(item.path))) {
  throw new Error("query or fragment leaked into a reconciled registry path");
}
NODE

  grep -q '^# Route inventory' "$OUTPUT/route-inventory.md"
  grep -q '/orphaned-nav' "$OUTPUT/route-inventory.md"
}

@test "unrelated cyclic data arrays do not block a valid route inventory" {
  run node "$SCRIPT" --repo "$FIXTURES/unrelated-cycle" --output-dir "$OUTPUT"

  [ "$status" -eq 0 ]
  node - "$OUTPUT/route-inventory.json" <<'NODE'
const fs = require("node:fs");
const inventory = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
if (inventory.routes.length !== 1 || inventory.routes[0].path !== "/alive") {
  throw new Error("valid route was not inventoried");
}
NODE
}

@test "cyclic route collection fails closed without artifacts" {
  run node "$SCRIPT" --repo "$FIXTURES/cycle" --output-dir "$OUTPUT"

  [ "$status" -eq 1 ]
  [[ "$output" == *"ROUTE_INVENTORY_CYCLE"* ]]
  [ ! -e "$OUTPUT/route-inventory.json" ]
  [ ! -e "$OUTPUT/route-inventory.md" ]
}

@test "single-harness uninstall retains helper until the last consumer is removed" {
  export HOME="$BATS_TEST_TMPDIR/home"
  export CLAUDE_CONFIG_DIR="$HOME/claude"
  export OPENCODE_CONFIG_DIR="$HOME/opencode"
  export CODEX_HOME="$HOME/codex"
  export AUTOSPEC_SCRIPTS_DIR="$HOME/autospec-scripts"
  local install="$REPO_ROOT/skills/autospec-ui-audit/install.sh"
  local uninstall="$REPO_ROOT/skills/autospec-ui-audit/uninstall.sh"
  local helper="$AUTOSPEC_SCRIPTS_DIR/autospec-ui-route-inventory.mjs"

  run bash "$install" --harness all
  [ "$status" -eq 0 ]
  [ -x "$helper" ]

  run bash "$uninstall" --harness codex
  [ "$status" -eq 0 ]
  [ -x "$helper" ]
  [ -f "$CLAUDE_CONFIG_DIR/skills/autospec-ui-audit/SKILL.md" ]
  [ -f "$OPENCODE_CONFIG_DIR/agent/autospec-ui-audit.md" ]

  run bash "$uninstall" --harness claude
  [ "$status" -eq 0 ]
  [ -x "$helper" ]

  run bash "$uninstall" --harness opencode
  [ "$status" -eq 0 ]
  [ ! -e "$helper" ]

  run bash "$install" --harness all
  [ "$status" -eq 0 ]
  run bash "$uninstall" --harness all
  [ "$status" -eq 0 ]
  [ ! -e "$helper" ]
}
