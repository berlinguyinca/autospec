#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
}

# Writes a complete, well-ordered portfolio transaction into the fixture tree:
# primary Project verified before the first issue create, every planned item
# bound exactly once, complete audit-inclusive parent records, and the
# cross-repository graph persisted after the last create.
install_transaction() {
    mkdir -p "$BATS_TEST_TMPDIR/.autospec/state"
    cat > "$BATS_TEST_TMPDIR/.autospec/state/portfolio-transaction.json" <<'JSON'
{
  "schema": "autospec.portfolio-transaction.v1",
  "portfolio_id": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "project_owner": "org",
  "plan_digest": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
  "state": "complete",
  "items": [
    {"item_key": "tracker:src", "repository": "org/src", "role": "umbrella"},
    {"item_key": "issue:alpha", "repository": "org/src", "role": "implementation"},
    {"item_key": "tracker:other", "repository": "org/other", "role": "tracker"},
    {"item_key": "issue:beta", "repository": "org/other", "role": "implementation"},
    {"item_key": "audit:phase-5.5", "repository": "org/src", "role": "audit"}
  ],
  "edges": [
    {"from": "issue:beta", "to": "issue:alpha"}
  ],
  "ops": [
    {"seq": 1, "op": "project.verified", "status": "acknowledged"},
    {"seq": 2, "op": "issue.create", "item_key": "tracker:src", "status": "acknowledged"},
    {"seq": 3, "op": "issue.create", "item_key": "issue:alpha", "status": "acknowledged"},
    {"seq": 4, "op": "issue.create", "item_key": "tracker:other", "status": "acknowledged"},
    {"seq": 5, "op": "issue.create", "item_key": "issue:beta", "status": "acknowledged"},
    {"seq": 6, "op": "issue.create", "item_key": "audit:phase-5.5", "status": "acknowledged"},
    {"seq": 7, "op": "parent.record", "status": "acknowledged"},
    {"seq": 8, "op": "parent.record", "status": "acknowledged"},
    {"seq": 9, "op": "graph.persisted", "status": "acknowledged"}
  ],
  "parent_records": [
    {"repository": "org/src", "parent": "tracker:src", "children": ["issue:alpha", "audit:phase-5.5"]},
    {"repository": "org/other", "parent": "tracker:other", "children": ["issue:beta"]}
  ]
}
JSON
}

# Applies one deterministic mutation to the installed transaction.
mutate_transaction() {
    local src="$BATS_TEST_TMPDIR/.autospec/state/portfolio-transaction.json"
    python3 - "$src" "$1" <<'PY'
import json, sys
path, name = sys.argv[1], sys.argv[2]
data = json.load(open(path, encoding="utf-8"))
ops = data.get("ops", [])
if name == "no-audit-child":
    for record in data["parent_records"]:
        if record["repository"] == "org/src":
            record["children"] = [c for c in record["children"] if c != "audit:phase-5.5"]
elif name == "create-before-verified":
    ops.sort(key=lambda op: op["op"] != "issue.create")
elif name == "graph-lost":
    ops[:] = [op for op in ops if op["op"] != "graph.persisted"]
elif name == "missing-create":
    ops[:] = [op for op in ops
              if not (op["op"] == "issue.create" and op.get("item_key") == "issue:beta")]
elif name == "duplicate-create":
    ops.append({"op": "issue.create", "item_key": "issue:alpha", "status": "acknowledged"})
for index, op in enumerate(ops, start=1):
    op["seq"] = index
json.dump(data, open(path, "w", encoding="utf-8"), indent=2)
PY
}

validate_state() {
    run bash "$REPO_ROOT/scripts/autospec-validate-state.sh" --repo-root "$BATS_TEST_TMPDIR"
}

@test "every decomposition workflow records the umbrella and child relationship" {
    local skill
    for skill in autospec-define autospec-split; do
        run grep -F 'parent record --repo {repo} --parent "<UMBRELLA>" --children "<CHILDREN_CSV>"' \
            "$REPO_ROOT/skills/$skill/SKILL.md"
        [ "$status" -eq 0 ]
    done
    # The end-to-end router owns no decomposition phase of its own; it must
    # delegate the define half so the umbrella/child record stays on the
    # definition paths above.
    run grep -F 'Delegate to /autospec-define' "$REPO_ROOT/skills/autospec/SKILL.md"
    [ "$status" -eq 0 ]
}

@test "implementation workflows reconcile the parent after the child merge" {
    run grep -F 'parent reconcile-child --repo {repo} --child "<ISSUE>"' \
        "$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ "$status" -eq 0 ]
    # The end-to-end router delegates its implementation half to autospec-run,
    # so post-merge reconciliation rides the same typed command.
    run grep -F 'Delegate to /autospec-run' "$REPO_ROOT/skills/autospec/SKILL.md"
    [ "$status" -eq 0 ]
}

@test "implementation workflows sweep parents closed outside autospec" {
    run grep -F 'parent sweep --repo {repo}' "$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ "$status" -eq 0 ]
}

@test "run workflow reserves umbrella mutation for the typed parent command" {
    run grep -F 'Only `autospec parent` may update or close an umbrella issue' \
        "$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ "$status" -eq 0 ]
}

@test "complete audit-inclusive parent set and admission ordering pass state validation" {
    install_transaction
    validate_state
    [ "$status" -eq 0 ]
    [[ "$output" == *"state validation: pass"* ]]
}

@test "partial parent record missing the audit child fails state validation" {
    install_transaction
    mutate_transaction no-audit-child
    validate_state
    [ "$status" -eq 1 ]
    run grep -F 'audit:phase-5.5' "$BATS_TEST_TMPDIR/.autospec/reports/state-validation.md"
    [ "$status" -eq 0 ]
}

@test "issue creation before Project verification fails admission ordering" {
    install_transaction
    mutate_transaction create-before-verified
    validate_state
    [ "$status" -eq 1 ]
    run grep -F 'before primary Project verification' \
        "$BATS_TEST_TMPDIR/.autospec/reports/state-validation.md"
    [ "$status" -eq 0 ]
}

@test "lost cross-repository graph persistence fails state validation" {
    install_transaction
    mutate_transaction graph-lost
    validate_state
    [ "$status" -eq 1 ]
    run grep -F 'graph not persisted' "$BATS_TEST_TMPDIR/.autospec/reports/state-validation.md"
    [ "$status" -eq 0 ]
}

@test "planned item never created fails complete transaction state" {
    install_transaction
    mutate_transaction missing-create
    validate_state
    [ "$status" -eq 1 ]
    run grep -F 'never created' "$BATS_TEST_TMPDIR/.autospec/reports/state-validation.md"
    [ "$status" -eq 0 ]
}

@test "duplicate issue binding for one item key fails complete transaction state" {
    install_transaction
    mutate_transaction duplicate-create
    validate_state
    [ "$status" -eq 1 ]
    run grep -F 'duplicate issue.create' "$BATS_TEST_TMPDIR/.autospec/reports/state-validation.md"
    [ "$status" -eq 0 ]
}
