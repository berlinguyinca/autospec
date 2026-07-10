#!/usr/bin/env bats
# Coverage for scripts/autonomous-promote-open-issues.sh — the Tier-1.5 grooming
# orchestrator: safety → classify → eligibility → promote/groom/split/hold,
# policy-gated. All sub-scripts and `gh` are stubbed; no live GitHub, no live LLM.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/autonomous-promote-open-issues.sh"
  GH_LOG="$TMP/gh.log"; export GH_LOG
  : > "$GH_LOG"

  # ── gh shim: `issue view` → fixture JSON; mutations → append argv to GH_LOG ──
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
case "$*" in
  *"issue view"*) cat "${GH_VIEW_JSON:-/dev/null}" ;;
  *) printf '%s\n' "$*" >> "$GH_LOG" ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"

  # ── Canonical single-candidate view fixture ─────────────────────────────────
  GH_VIEW_JSON="$TMP/view.json"; export GH_VIEW_JSON
  cat > "$GH_VIEW_JSON" <<'JSON'
{"number":42,"title":"fix: crash","body":"fix: guard the loop; repro: run empty backlog. Expected: no crash.","labels":[{"name":"needs-classify"}]}
JSON

  # ── list-groomable stub: one needs-classify candidate ───────────────────────
  cat > "$TMP/bin/list.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' '{"candidates":[{"number":42,"title":"fix: crash","class":"needs-classify"}],"skipped":[]}'
SH
  chmod +x "$TMP/bin/list.sh"
  export AUTOSPEC_GROOM_LIST_SCRIPT="$TMP/bin/list.sh"

  # ── classify stub → ctx/reasoning JSON ──────────────────────────────────────
  cat > "$TMP/bin/classify.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' '{"ctx":"64k","reasoning":"medium"}'
SH
  chmod +x "$TMP/bin/classify.sh"
  export AUTOSPEC_GROOM_CLASSIFY_SCRIPT="$TMP/bin/classify.sh"

  # ── govern stub → active set (default: template-promote NOT active) ──────────
  cat > "$TMP/bin/govern.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "${GROOM_GOVERN_ACTIVE:-{\"active\":[\"eligible-promote\"]}}"
SH
  chmod +x "$TMP/bin/govern.sh"
  export AUTOSPEC_GROOM_GOVERN_SCRIPT="$TMP/bin/govern.sh"

  # ── safety + eligibility stubs are per-test (below) ─────────────────────────
  mk_safety() { printf '#!/usr/bin/env bash\nprintf %%s\\\\n "%s"\n' "$1" > "$TMP/bin/safety.sh"; chmod +x "$TMP/bin/safety.sh"; export AUTOSPEC_GROOM_SAFETY_SCRIPT="$TMP/bin/safety.sh"; }
  mk_elig() { printf '#!/usr/bin/env bash\nprintf %%s\\\\n %s\n' "'{\"decision\":\"$1\",\"reason\":\"stub\"}'" > "$TMP/bin/elig.sh"; chmod +x "$TMP/bin/elig.sh"; export AUTOSPEC_GROOM_ELIGIBILITY_SCRIPT="$TMP/bin/elig.sh"; }

  # Force config defaults (policy=auto) — no repo autospec.yml bleed-through.
  export AUTOSPEC_CONFIG_FILE="$TMP/nonexistent.yml"
}
teardown() { rm -rf "$TMP"; }

@test "eligible needs-classify issue is promoted with audit comment" {
  mk_safety "SAFETY_PASS"; mk_elig "eligible"
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label auto-implement' "$GH_LOG"
  grep -q 'remove-label needs-classify' "$GH_LOG"
  grep -q 'issue comment' "$GH_LOG"
}

@test "SAFETY_BLOCK quarantines and never promotes" {
  mk_safety "SAFETY_BLOCK"; mk_elig "eligible"
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label security:quarantined' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
}

@test "needs-template held when template-promote not in active govern set" {
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote"]}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label hold:needs-human' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
}

@test "policy off mutates nothing" {
  mk_safety "SAFETY_PASS"; mk_elig "eligible"
  AUTOSPEC_GROOMING_POLICY=off run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  [ ! -s "$GH_LOG" ] || ! grep -q 'issue edit' "$GH_LOG"
  echo "$output" | jq -e '.dry == true'
}
