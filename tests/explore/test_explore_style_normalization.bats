#!/usr/bin/env bats
# tests/explore/test_explore_style_normalization.bats — style-normalization
# researcher contract: frontend style-drift proposals require objective
# Playwright + screenshot proof before filing.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t style-normalization.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
}

teardown() {
    rm -rf "$TMP"
}

assert_well_formed() {
    local json="$1"
    printf '%s' "$json" | python3 -c '
import json, sys
d = json.load(sys.stdin)
assert d["source"] == "style-normalization", d
assert isinstance(d.get("proposals"), list), d
for p in d["proposals"]:
    for k in ("title", "evidence", "estimated_complexity", "confidence", "severity", "named_consumer"):
        assert k in p, f"proposal missing {k}"
'
}

seed_frontend_style_drift() {
    mkdir -p src
    cat > package.json <<'EOF'
{"dependencies":{"react":"latest","vite":"latest"}}
EOF
    cat > src/App.jsx <<'EOF'
export function App() {
  return (
    <main style={{ color: "#ff00aa", padding: "13px", borderRadius: "19px" }}>
      <button style={{ background: "#00ffbb", padding: "7px 11px" }}>Save</button>
      <input style={{ borderColor: "#ff00aa", margin: "3px" }} />
    </main>
  );
}
EOF
    git add -A && git commit -q -m "seed frontend style drift"
}

@test "style-normalization invokes proof command and emits proof-backed proposal" {
    seed_frontend_style_drift
    export AUTOSPEC_EXPLORE_ROUND=round-7
    export AUTOSPEC_EXPLORE_STYLE_PROOF_CMD='mkdir -p "$AUTOSPEC_STYLE_PROOF_DIR"; printf "%s\n" "import { test } from '"'"'@playwright/test'"'"';" "test('"'"'route visual baseline'"'"', async ({ page }) => { await page.goto('"'"'/'"'"'); });" > "$AUTOSPEC_STYLE_PROOF_DIR/style.spec.ts"; printf "png" > "$AUTOSPEC_STYLE_PROOF_DIR/home.png"; printf "proof-ran" > "$AUTOSPEC_STYLE_PROOF_DIR/ran.txt"'

    run bash "$REPO_ROOT/scripts/explore-research/style-normalization.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"style-normalization"* ]]
    [[ "$output" == *"style.spec.ts"* ]]
    [[ "$output" == *"home.png"* ]]
    [ -f "$TMP/.autospec/style-normalization/round-7/ran.txt" ]
}

@test "style-normalization does not file style proposal without Playwright and screenshot proof" {
    seed_frontend_style_drift
    unset AUTOSPEC_EXPLORE_STYLE_PROOF_CMD

    run bash "$REPO_ROOT/scripts/explore-research/style-normalization.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    printf '%s' "$output" | python3 -c '
import json, sys
d = json.load(sys.stdin)
assert d["proposals"] == [], d
'
}

@test "style-normalization emits empty proposals for non-frontend repos" {
    mkdir -p scripts
    echo 'echo hello' > scripts/tool.sh
    git add -A && git commit -q -m "seed shell repo"

    run bash "$REPO_ROOT/scripts/explore-research/style-normalization.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    printf '%s' "$output" | python3 -c '
import json, sys
d = json.load(sys.stdin)
assert d["proposals"] == [], d
'
}
