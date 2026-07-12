#!/usr/bin/env bats
# tests/autonomous/test_design_gates.bats — baseline-pack design gate runner
# (autospec-design-gates.sh) and its opt-in premerge stage.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
RUNNER="$REPO_ROOT/scripts/autospec-design-gates.sh"
PREMERGE="$REPO_ROOT/scripts/autonomous-premerge-gate.sh"

setup() {
    TMP="$(mktemp -d -t design_gates.XXXXXX)"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin" "$TMP/repo/.autospec" "$TMP/repo/pack"

    cat > "$TMP/bin/autospec-qa" <<'STUB'
#!/usr/bin/env bash
printf 'autospec-qa: all checks passed\n'
STUB
    chmod +x "$TMP/bin/autospec-qa"

    cat > "$TMP/bin/autospec-secaudit" <<'STUB'
#!/usr/bin/env bash
printf 'secaudit: must-fix=0\n'
STUB
    chmod +x "$TMP/bin/autospec-secaudit"

    cat > "$TMP/repo/pack/rules.yaml" <<'YAML'
version: 0.5.0
rules:
- id: tokens.dtcg-valid
  severity: blocker
  check: auto
  tool: ajv
  pass: tokens.json validates against the DTCG schema.
- id: a11y.axe-clean
  severity: blocker
  check: auto
  tool: axe-core
  pass: no critical or serious axe violations.
- id: a11y.target-size
  severity: minor
  check: auto
  tool: hit-area check
  pass: interactive targets meet minimum size.
- id: a11y.no-color-only-status
  severity: major
  check: vlm
  tool: VLM critique
  pass: no status by color alone.
YAML

    cat > "$TMP/repo/pack/web.pack.json" <<'JSON'
{"qualityGates": ["Contrast meets AA in every theme.", "No color-only status."]}
JSON

    cat > "$TMP/repo/.autospec/design-gates.yml" <<'YAML'
rules_file: pack/rules.yaml
pack_file: pack/web.pack.json
ui_paths:
  - "src/**"
  - "**/*.css"
gates:
  tokens.dtcg-valid:
    command: "true"
  a11y.axe-clean:
    command: "true"
YAML
    git -C "$TMP/repo" init -q
    git -C "$TMP/repo" checkout -q -b design-gates-test
}

teardown() {
    rm -rf "$TMP"
}

@test "runner: no config skips with exit 0" {
    bare="$(mktemp -d -t design_gates_bare.XXXXXX)"
    run -0 "$RUNNER" --repo-root "$bare"
    [[ "$output" == *"autospec-design-gates: SKIPPED"* ]]
    run -0 python3 -c "import json;print(json.load(open('$bare/.autospec/reports/design-gates.json'))['reason'])"
    [ "$output" = "no-config" ]
    rm -rf "$bare"
}

@test "runner: non-UI changed files skip via ui_paths" {
    printf 'docs/readme.md\nscripts/tool.sh\n' > "$TMP/changed.txt"
    run -0 "$RUNNER" --repo-root "$TMP/repo" --changed-files "$TMP/changed.txt"
    [[ "$output" == *"SKIPPED"* ]]
    run -0 python3 -c "import json;print(json.load(open('$TMP/repo/.autospec/reports/design-gates.json'))['reason'])"
    [ "$output" = "skipped_not_ui" ]
}

@test "runner: UI-touching changed files run the mapped gates" {
    printf 'src/app/page.tsx\n' > "$TMP/changed.txt"
    run -0 "$RUNNER" --repo-root "$TMP/repo" --changed-files "$TMP/changed.txt"
    [[ "$output" == *"autospec-design-gates: PASS (2 run, 0 failed, 1 unmapped)"* ]]
}

@test "runner: failing blocker gate exits 1 with FAIL status line" {
    cat > "$TMP/repo/.autospec/design-gates.yml" <<'YAML'
rules_file: pack/rules.yaml
gates:
  tokens.dtcg-valid:
    command: "echo raw literal found; exit 1"
  a11y.axe-clean:
    command: "true"
YAML
    run -1 "$RUNNER" --repo-root "$TMP/repo"
    [[ "$output" == *"autospec-design-gates: FAIL (2 run, 1 failed, 1 unmapped)"* ]]
    run -0 python3 -c "import json;d=json.load(open('$TMP/repo/.autospec/reports/design-gates.json'));print([g['status'] for g in d['gates'] if g['id']=='tokens.dtcg-valid'][0])"
    [ "$output" = "fail" ]
}

@test "runner: failing non-blocking gate stays exit 0" {
    cat > "$TMP/repo/.autospec/design-gates.yml" <<'YAML'
rules_file: pack/rules.yaml
gates:
  a11y.target-size:
    command: "exit 1"
YAML
    run -0 "$RUNNER" --repo-root "$TMP/repo"
    [[ "$output" == *"autospec-design-gates: PASS (1 run, 1 failed, 2 unmapped)"* ]]
}

@test "runner: unmapped auto rules are advisory unless --strict" {
    cat > "$TMP/repo/.autospec/design-gates.yml" <<'YAML'
rules_file: pack/rules.yaml
gates:
  tokens.dtcg-valid:
    command: "true"
YAML
    run -0 "$RUNNER" --repo-root "$TMP/repo"
    run -1 "$RUNNER" --repo-root "$TMP/repo" --strict
    [[ "$output" == *"FAIL"* ]]
}

@test "runner: report carries critic checklist and pack quality gates" {
    run -0 "$RUNNER" --repo-root "$TMP/repo"
    md="$TMP/repo/.autospec/reports/design-gates.md"
    grep -q 'a11y.no-color-only-status' "$md"
    grep -q 'Contrast meets AA in every theme.' "$md"
    run -0 python3 -c "import json;d=json.load(open('$TMP/repo/.autospec/reports/design-gates.json'));print(len(d['pack_quality_gates']))"
    [ "$output" = "2" ]
}

@test "runner: unknown rule id in gates config is a config error (exit 2)" {
    cat > "$TMP/repo/.autospec/design-gates.yml" <<'YAML'
rules_file: pack/rules.yaml
gates:
  no.such.rule:
    command: "true"
YAML
    run -2 "$RUNNER" --repo-root "$TMP/repo"
    [[ "$output" == *"unknown rule ids"* ]]
}

@test "premerge: failing design gate blocks after retries" {
    cat > "$TMP/repo/.autospec/design-gates.yml" <<'YAML'
rules_file: pack/rules.yaml
gates:
  tokens.dtcg-valid:
    command: "exit 1"
YAML
    cd "$TMP/repo"
    AUTOSPEC_NOTIFY=0 AUTOSPEC_REPO_DIR="$TMP/repo" \
        run -1 bash "$PREMERGE" --pr-branch design-gates-test --max-attempts 2
    [[ "$output" == *"block retries_exhausted"* ]]
    [[ "$output" == *"design-gates run (attempt 2/2)"* ]]
}

@test "premerge: passing design gate proceeds to QA and merge-ok" {
    cd "$TMP/repo"
    AUTOSPEC_NOTIFY=0 AUTOSPEC_REPO_DIR="$TMP/repo" \
        run -0 bash "$PREMERGE" --pr-branch design-gates-test --max-attempts 2
    [[ "$output" == *"design-gates stage clear"* ]]
    [[ "$output" == *"merge-ok"* ]]
}

@test "premerge: absent config skips the stage and still merges" {
    rm "$TMP/repo/.autospec/design-gates.yml"
    cd "$TMP/repo"
    AUTOSPEC_NOTIFY=0 AUTOSPEC_REPO_DIR="$TMP/repo" \
        run -0 bash "$PREMERGE" --pr-branch design-gates-test --max-attempts 2
    [[ "$output" == *"no .autospec/design-gates.yml; skipping stage"* ]]
    [[ "$output" == *"merge-ok"* ]]
}

@test "premerge: dry-run reports the stage without executing" {
    cd "$TMP/repo"
    AUTOSPEC_NOTIFY=0 AUTOSPEC_REPO_DIR="$TMP/repo" \
        run -0 bash "$PREMERGE" --pr-branch design-gates-test --dry-run
    [[ "$output" == *"[dry-run] would run:"* ]]
    [[ "$output" == *"merge-ok"* ]]
}
