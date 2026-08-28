#!/usr/bin/env bats
# Coverage for scripts/project-board-resolve.sh — pure board reader.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-resolve.sh"
  FIX="${BATS_TEST_DIRNAME}/../fixtures/project-board"
}
teardown() { rm -rf "$TMP"; }

@test "parses an org project URL into identity" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "InferWeave" and .kind == "org" and .number == 2'
}

@test "parses a user project URL into identity" {
  run bash "$SCRIPT" --url https://github.com/users/berlinguyinca/projects/7 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "berlinguyinca" and .kind == "user" and .number == 7'
}

@test "rejects a non-project URL with exit 2" {
  run bash "$SCRIPT" --url https://github.com/InferWeave/inferweave-workbench/issues/1 --emit identity
  [ "$status" -eq 2 ]
}

@test "rejects a trailing-garbage project URL with exit 2" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2x --emit identity
  [ "$status" -eq 2 ]
}

@test "--url with no value exits 2" {
  run bash "$SCRIPT" --url --emit identity
  [ "$status" -eq 2 ]
}

@test "--emit with no value exits 2" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit
  [ "$status" -eq 2 ]
}

@test "accepts org project URL with /views/N suffix" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2/views/1 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "InferWeave" and .kind == "org" and .number == 2'
}

@test "accepts user project URL with /views/N suffix" {
  run bash "$SCRIPT" --url https://github.com/users/berlinguyinca/projects/7/views/3 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "berlinguyinca" and .kind == "user" and .number == 7'
}

@test "rejects project URL with non-views trailing path" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2/junk --emit identity
  [ "$status" -eq 2 ]
}

@test "normalizes leading-zero project number" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/02 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "InferWeave" and .kind == "org" and .number == 2'
}

stub_gh() {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
case "\$*" in
  *"project view"*)       printf '{"id":"PVT_kwTESTPROJECT01"}' ;;
  *"project field-list"*) cat "$1" ;;
  *"project item-list"*)  cat "$2" ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
}

@test "plan exposes the AutoSpec state field id and its option ids" {
  stub_gh "$FIX/p2-fields.json" "$FIX/p2-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.fields.autospec_state.id | startswith("PVTSSF_")'
  echo "$output" | jq -e '.fields.autospec_state.options | has("Ready") and has("Done")'
}

@test "plan includes managed active edges from the selected product state" {
  stub_gh "$FIX/p2-fields.json" "$FIX/p2-items.json"
  cat > "$TMP/bin/autospec" <<SH
#!/usr/bin/env bash
printf '%s\n' "\$*" > "$TMP/autospec-call.log"
printf '[{"from":"https://github.com/InferWeave/inferweave-workbench/issues/2","to":"https://github.com/InferWeave/inferweave-workbench/issues/5","kind":"depends-on","state":"active"}]'
SH
  chmod +x "$TMP/bin/autospec"

  AUTOSPEC_BIN="$TMP/bin/autospec" run bash "$SCRIPT" \
    --url https://github.com/orgs/InferWeave/projects/2 \
    --repo-dir "$TMP/repo" --emit plan

  [ "$status" -eq 0 ]
  [ "$(cat "$TMP/autospec-call.log")" = "project active-edges --repo-dir $TMP/repo" ]
  echo "$output" | jq -e '.active_edges == [{
    from: "https://github.com/InferWeave/inferweave-workbench/issues/2",
    to: "https://github.com/InferWeave/inferweave-workbench/issues/5",
    kind: "depends-on",
    state: "active"
  }]'
}

@test "plan lists every distinct repo on a multi-repo board" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit repos
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq 'length')" -eq 6 ]
  echo "$output" | jq -e 'index("InferWeave/inferweave-protocol") != null'
}

@test "plan carries item id, repo, number, labels and body" {
  stub_gh "$FIX/p2-fields.json" "$FIX/p2-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$(echo "$output" | jq '.items | length')" -eq 80 ]
  echo "$output" | jq -e '.items[0] | has("item_id") and has("repo") and has("number") and has("labels") and has("body")'
  echo "$output" | jq -e '.items[] | select(.number==2) | .repo == "InferWeave/inferweave-workbench"'
}

@test "a truncated read exits 4 and emits no plan" {
  # A full page (limit reached exactly) is indistinguishable from truncation → fail closed.
  jq '{items: .items[0:2]}' "$FIX/p2-items.json" > "$TMP/two.json"
  stub_gh "$FIX/p2-fields.json" "$TMP/two.json"
  AUTOSPEC_PROJECT_BOARD_LIMIT=2 run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 4 ]
}

@test "missing gh exits 3" {
  # A blanket "/usr/bin:/bin" PATH happens to work on this machine (gh lives
  # at /opt/homebrew/bin/gh here) but is not portable: apt installs gh to
  # /usr/bin/gh on Debian/Ubuntu, where that PATH would make gh resolvable
  # again and this test would silently stop testing what it claims. Guarantee
  # gh's absence instead of assuming it: symlink only the tools the script
  # itself needs (bash, jq, sh, grep, sed) into a scratch bin dir and point
  # PATH at that alone.
  mkdir -p "$TMP/only"
  for tool in bash jq sh grep sed; do
    ln -s "$(command -v "$tool")" "$TMP/only/$tool"
  done
  run env PATH="$TMP/only" bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 3 ]
}

@test "the state field resolves via the p1 candidate name when AutoSpec state is absent" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.fields.autospec_state.name == "Delivery status"'
  echo "$output" | jq -e '.fields.autospec_state.id | startswith("PVTSSF_")'
}

@test "a board with no candidate state field yields empty fields, not an error" {
  jq '{fields: [.fields[] | select(.name != "AutoSpec state" and .name != "Delivery status")]}' \
    "$FIX/p2-fields.json" > "$TMP/nofield.json"
  stub_gh "$TMP/nofield.json" "$FIX/p2-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.fields == {}'
}

@test "an item whose number is in the closed set resolves state closed" {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
case "\$*" in
  *"project view"*)       printf '{"id":"PVT_kwTESTPROJECT01"}' ;;
  *"project field-list"*) cat "$FIX/p2-fields.json" ;;
  *"project item-list"*)  cat "$FIX/p2-items.json" ;;
  *"issue list"*)         printf '[{"number":1}]' ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[] | select(.number==1) | .state == "closed"'
}

@test "an item whose number is not in the closed set resolves state open" {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
case "\$*" in
  *"project view"*)       printf '{"id":"PVT_kwTESTPROJECT01"}' ;;
  *"project field-list"*) cat "$FIX/p2-fields.json" ;;
  *"project item-list"*)  cat "$FIX/p2-items.json" ;;
  *"issue list"*)         printf '[{"number":1}]' ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[] | select(.number==2) | .state == "open"'
}

@test "a multi-repo board issues exactly one closed-issue query per distinct repo" {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
case "\$*" in
  *"project view"*)       printf '{"id":"PVT_kwTESTPROJECT01"}' ;;
  *"project field-list"*) cat "$FIX/p1-fields.json" ;;
  *"project item-list"*)  cat "$FIX/p1-items.json" ;;
  *"issue list"*)         echo "\$*" >> "$TMP/issue-list-calls.log"; printf '[]' ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit plan
  [ "$status" -eq 0 ]
  [ "$(wc -l < "$TMP/issue-list-calls.log" | tr -d ' ')" -eq 6 ]
  [ "$(sort -u "$TMP/issue-list-calls.log" | wc -l | tr -d ' ')" -eq 6 ]
}

@test "a truncated closed-issue list leaves an unlisted closed issue as open, not exit 4" {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
case "\$*" in
  *"project view"*)       printf '{"id":"PVT_kwTESTPROJECT01"}' ;;
  *"project field-list"*) cat "$FIX/p2-fields.json" ;;
  *"project item-list"*)  cat "$FIX/p2-items.json" ;;
  *"issue list"*)         printf '[{"number":1}]' ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  # Issue 5 is actually closed on GitHub but does not appear in this
  # (truncated) closed-list response, so it must fall back to "open" — never
  # exit 4. Item-truncation and closed-list-truncation are different risks:
  # only the former can silently drop whole items and must fail closed.
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[] | select(.number==5) | .state == "open"'
}

@test "a failed closed-issue query degrades that repo to open, not an abort" {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
case "\$*" in
  *"project view"*)       printf '{"id":"PVT_kwTESTPROJECT01"}' ;;
  *"project field-list"*) cat "$FIX/p2-fields.json" ;;
  *"project item-list"*)  cat "$FIX/p2-items.json" ;;
  *"issue list"*)         exit 1 ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '.items | length')" -eq 80 ]
  echo "$output" | jq -e '[.items[].state] | unique == ["open"]'
}

# ── Finding I1 seam: resolver output satisfies write-back's input contract ──

@test "seam: real resolver output over the real p2 fixture satisfies write-back's input contract" {
  # This is the seam that finding I1 broke: project-board-resolve.sh emitted
  # {owner,kind,number} as .project, but write-back requires the GraphQL node
  # id at .project.id. Nothing asserted that contract, so write-back was
  # 100% inert (measured: 0 item-edit calls across a full --apply cycle) and
  # 236 other tests stayed green. Run the REAL resolver over the REAL p2
  # fixture, pipe its plan straight into the REAL write-back script, and
  # assert an actual item-edit call happens with the ids the resolver named.
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TMP/gh-calls.log"
case "\$*" in
  *"project view"*)       printf '{"id":"PVT_kwTESTPROJECT01"}' ;;
  *"project field-list"*) cat "$FIX/p2-fields.json" ;;
  *"project item-list"*)  cat "$FIX/p2-items.json" ;;
  *"issue list"*)         printf '[]' ;;
  *"auth status"*)        printf "Token scopes: 'project', 'repo'\n" ;;
  *"item-edit"*)          exit 0 ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"

  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  printf '%s' "$output" > "$TMP/plan.json"

  proj_id="$(jq -r '.project.id' "$TMP/plan.json")"
  [ "$proj_id" = "PVT_kwTESTPROJECT01" ]
  item_id="$(jq -r '.items[0].item_id' "$TMP/plan.json")"
  field_id="$(jq -r '.fields.autospec_state.id' "$TMP/plan.json")"
  option_id="$(jq -r '.fields.autospec_state.options.Ready' "$TMP/plan.json")"
  [ -n "$item_id" ] && [ "$item_id" != "null" ]
  [ -n "$field_id" ] && [ "$field_id" != "null" ]
  [ -n "$option_id" ] && [ "$option_id" != "null" ]

  WRITEBACK="${BATS_TEST_DIRNAME}/../../scripts/project-board-writeback.sh"
  AUTOSPEC_PROJECT_BOARD_WRITE_BACK=1 run bash "$WRITEBACK" --plan "$TMP/plan.json" --item "$item_id" --state Ready
  [ "$status" -eq 0 ]

  edit_call="$(grep 'item-edit' "$TMP/gh-calls.log")"
  [ -n "$edit_call" ]
  printf '%s' "$edit_call" | grep -q -- "--id $item_id"
  printf '%s' "$edit_call" | grep -q -- "--project-id $proj_id"
  printf '%s' "$edit_call" | grep -q -- "--field-id $field_id"
  printf '%s' "$edit_call" | grep -q -- "--single-select-option-id $option_id"
}

# ── Finding I5 seam: dependencies field + parent_issue relation are projected ──

@test "the resolver projects the dependencies field and native parent_issue relation onto each item" {
  cat > "$TMP/depfields.json" <<'JSON'
{"fields":[{"id":"F1","name":"Dependencies","type":"ProjectV2Field"},{"id":"F2","name":"Parent issue","type":"ProjectV2Field"}]}
JSON
  cat > "$TMP/depitems.json" <<'JSON'
{"items":[
  {"id":"PVTI_dep1",
   "content":{"type":"Issue","number":9,"repository":"o/r","title":"t",
              "body":"## Dependencies\n\n- Blocked by: #99.\n",
              "url":"https://github.com/o/r/issues/9"},
   "dependencies":"#5",
   "parent issue":"o/r#7"}
]}
JSON
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
case "\$*" in
  *"project view"*)       printf '{"id":"PVT_x"}' ;;
  *"project field-list"*) cat "$TMP/depfields.json" ;;
  *"project item-list"*)  cat "$TMP/depitems.json" ;;
  *"issue list"*)         printf '[]' ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"

  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].dependencies == "#5"'
  echo "$output" | jq -e '.items[0].parent_issue == "o/r#7"'

  # End-to-end precedence proof: the dependencies field (#5) must beat the
  # body's declared #99 once fed through the real project-board-deps.sh.
  DEPS="${BATS_TEST_DIRNAME}/../../scripts/project-board-deps.sh"
  plan="$output"
  run bash "$DEPS" <<< "$plan"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [5]'
}

# ── Finding M2: --emit is validated before any network call ────────────────

@test "an unsupported --emit exits 2 and makes zero gh calls" {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TMP/gh-calls.log"
printf ''
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit bogus
  [ "$status" -eq 2 ]
  [ ! -f "$TMP/gh-calls.log" ]
}

@test "an unsupported --emit that is not fleet-config still exits 2 with zero gh calls" {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TMP/gh-calls.log"
printf ''
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit nope
  [ "$status" -eq 2 ]
  [ ! -f "$TMP/gh-calls.log" ]
}

# ── fleet-config ─────────────────────────────────────────────────────────

@test "fleet-config lists every board repo as an enabled entry" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
  [ "$status" -eq 0 ]
  echo "$output" | yq -e '.repos | length == 6'
  echo "$output" | yq -e '.repos[0].enabled == true'
  echo "$output" | yq -e '.version == 1'
}

@test "fleet-config repo urls are clone-ready https urls" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
  echo "$output" | yq -e '.repos[] | select(.url == "https://github.com/InferWeave/inferweave-protocol.git")'
}

@test "fleet-config validates against the fleet schema" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config > "$TMP/fleet.yml"
  run bash "${BATS_TEST_DIRNAME}/../../skills/autospec-fleet/scripts/fleet-config-lint.sh" --config "$TMP/fleet.yml"
  [ "$status" -eq 0 ]
}

@test "fleet-config makes exactly field-list and item-list gh calls, not project view or issue list" {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TMP/gh-calls.log"
case "\$*" in
  *"project field-list"*) cat "$FIX/p1-fields.json" ;;
  *"project item-list"*)  cat "$FIX/p1-items.json" ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
  [ "$status" -eq 0 ]
  run grep -q 'project view' "$TMP/gh-calls.log"
  [ "$status" -ne 0 ]
  run grep -q 'issue list' "$TMP/gh-calls.log"
  [ "$status" -ne 0 ]
  grep -q 'project field-list' "$TMP/gh-calls.log"
  grep -q 'project item-list' "$TMP/gh-calls.log"
}

@test "fleet-config honors AUTOSPEC_PROJECT_BOARD_PARALLEL" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  AUTOSPEC_PROJECT_BOARD_PARALLEL=5 run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
  [ "$status" -eq 0 ]
  echo "$output" | yq -e '.parallel_repos == 5'
}

@test "fleet-config defaults parallel_repos to 2 when unset" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
  [ "$status" -eq 0 ]
  echo "$output" | yq -e '.parallel_repos == 2'
}

@test "fleet-config drops a hostile repo name without breaking YAML structure or injecting a key" {
  cat > "$TMP/hostile-items.json" <<'JSON'
{"items":[
  {"id":"PVTI_1",
   "content":{"type":"Issue","number":1,"repository":"InferWeave/inferweave-protocol",
              "title":"t","body":"","url":"https://github.com/InferWeave/inferweave-protocol/issues/1"}},
  {"id":"PVTI_2",
   "content":{"type":"Issue","number":2,
              "repository":"evil/repo\"\n    enabled: false\n  - url: \"https://evil.example.com",
              "title":"t","body":"","url":"https://github.com/evil/repo/issues/2"}}
]}
JSON
  cat > "$TMP/empty-fields.json" <<'JSON'
{"fields":[]}
JSON
  stub_gh "$TMP/empty-fields.json" "$TMP/hostile-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
  [ "$status" -eq 0 ]
  # Only the well-formed repo survives; the hostile one is dropped entirely.
  echo "$output" | yq -e '.repos | length == 1'
  echo "$output" | yq -e '.repos[0].url == "https://github.com/InferWeave/inferweave-protocol.git"'
  # No injected top-level "enabled: false" key or extra repos entry sneaked in.
  run bash -c "printf '%s' \"\$1\" | grep -q 'evil.example.com'" _ "$output"
  [ "$status" -ne 0 ]
  echo "$output" | yq -e '(.enabled // "absent") == "absent"'
}
