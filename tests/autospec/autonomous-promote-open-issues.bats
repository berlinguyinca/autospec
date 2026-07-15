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
  mk_safety() { printf '#!/usr/bin/env bash\nprintf %%s\\\\n "%s"\n' "$1" > "$TMP/bin/safety.sh"; chmod +x "$TMP/bin/safety.sh"; export AUTOSPEC_GROOM_SAFETY_BIN="$TMP/bin/safety.sh"; }
  mk_elig() { printf '#!/usr/bin/env bash\nprintf %%s\\\\n %s\n' "'{\"decision\":\"$1\",\"reason\":\"stub\"}'" > "$TMP/bin/elig.sh"; chmod +x "$TMP/bin/elig.sh"; export AUTOSPEC_GROOM_ELIGIBILITY_SCRIPT="$TMP/bin/elig.sh"; }
  # groom-fill stub seam: mk_fill ok → {"ok":true,"body":...}; mk_fill fail → {"ok":false,"reason":...}
  mk_fill() {
    if [ "$1" = "ok" ]; then
      printf '#!/usr/bin/env bash\nprintf %%s\\\\n %s\n' "'{\"ok\":true,\"body\":\"## Summary\\nFilled template body.\\n\"}'" > "$TMP/bin/fill.sh"
    elif [ "$1" = "ok-no-body" ]; then
      printf '#!/usr/bin/env bash\nprintf %%s\\\\n %s\n' "'{\"ok\":true}'" > "$TMP/bin/fill.sh"
    else
      printf '#!/usr/bin/env bash\nprintf %%s\\\\n %s\n' "'{\"ok\":false,\"reason\":\"attempts-exhausted\"}'" > "$TMP/bin/fill.sh"
    fi
    chmod +x "$TMP/bin/fill.sh"; export AUTOSPEC_GROOM_FILL_SCRIPT="$TMP/bin/fill.sh"
  }
  # override the single-candidate view fixture with a custom label set
  mk_view_labels() {
    cat > "$GH_VIEW_JSON" <<JSON
{"number":42,"title":"fix: crash","body":"fix: guard the loop; repro: run empty backlog. Expected: no crash.","labels":[$1]}
JSON
  }

  # apply-safety-review stub seam: mk_apply_safety "pass" (exit 0, logs
  # add-label safety:reviewed) or "quarantine" (exit 1, logs
  # add-label security:quarantined).
  mk_apply_safety() {
    if [ "$1" = "pass" ]; then
      cat > "$TMP/bin/applysafety.sh" <<'SH'
#!/usr/bin/env bash
# log the mutation intent so tests can assert it
printf '%s\n' "apply-safety add-label safety:reviewed issue" >> "$GH_LOG"
printf '%s\n' '{"decision":"SAFETY_PASS","stamped":true}'
exit 0
SH
    else
      cat > "$TMP/bin/applysafety.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "apply-safety add-label security:quarantined issue" >> "$GH_LOG"
printf '%s\n' '{"decision":"SAFETY_AMBIGUOUS","stamped":false}'
exit 1
SH
    fi
    chmod +x "$TMP/bin/applysafety.sh"
    export AUTOSPEC_GROOM_APPLY_SAFETY_SCRIPT="$TMP/bin/applysafety.sh"
  }

  # Force config defaults (policy=auto) — no repo autospec.yml bleed-through.
  export AUTOSPEC_CONFIG_FILE="$TMP/nonexistent.yml"
}
teardown() { rm -rf "$TMP"; }

@test "eligible needs-classify issue is promoted with audit comment" {
  mk_safety "SAFETY_PASS"; mk_elig "eligible"; mk_apply_safety "pass"
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

@test "needs-template + seed state → canary: groom:proposed, no auto-implement" {
  # Semantic change vs v1: needs-template with the template-promote gate NOT
  # active no longer holds — it template-fills (canary) and proposes for human
  # approval. The hold path now only triggers on fill failure (see below).
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"; mk_fill ok; mk_apply_safety "pass"
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote"]}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label groom:proposed' "$GH_LOG"
  grep -q 'remove-label needs-autospec-template' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
  printf '%s' "$output" | jq -e '.routed[] | select(.action=="groom-canary")' >/dev/null
}

@test "needs-template + template-promote active → auto: auto-implement, no groom:proposed" {
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"; mk_fill ok; mk_apply_safety "pass"
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote","template-promote"]}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label auto-implement' "$GH_LOG"
  grep -q 'remove-label needs-autospec-template' "$GH_LOG"
  ! grep -q 'add-label groom:proposed' "$GH_LOG"
  printf '%s' "$output" | jq -e '.routed[] | select(.action=="groom-auto")' >/dev/null
}

@test "needs-template + fill fails → hold:needs-human (no promote)" {
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"; mk_fill fail
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote","template-promote"]}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label hold:needs-human' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
  ! grep -q 'add-label groom:proposed' "$GH_LOG"
  printf '%s' "$output" | jq -e '.held[] | select(.reason | test("fill-"))' >/dev/null
}

@test "needs-template + fill ok:true with no body → hold:needs-human (no promote)" {
  # Contract violation: fill stub returns {"ok":true} with no body field.
  # jq -r '.body' on that yields the literal string "null" — must not be
  # written as the issue body or promoted, even in graduated (auto) mode.
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"; mk_fill ok-no-body
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote","template-promote"]}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label hold:needs-human' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
  ! grep -q 'add-label groom:proposed' "$GH_LOG"
  printf '%s' "$output" | jq -e '.held[] | select(.reason | test("fill-empty-body"))' >/dev/null
}

@test "already groom:proposed candidate is skipped (no re-fill)" {
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"; mk_fill ok
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote"]}'
  mk_view_labels '{"name":"needs-autospec-template"},{"name":"groom:proposed"}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  ! grep -q 'add-label groom:proposed' "$GH_LOG"   # not re-proposed
  ! grep -q 'add-label auto-implement' "$GH_LOG"
  printf '%s' "$output" | jq -e '.skipped[] | select(.reason=="already-groomed")' >/dev/null
}

@test "policy off mutates nothing" {
  mk_safety "SAFETY_PASS"; mk_elig "eligible"
  AUTOSPEC_GROOMING_POLICY=off run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  [ ! -s "$GH_LOG" ] || ! grep -q 'issue edit' "$GH_LOG"
  echo "$output" | jq -e '.dry == true'
}

@test "canary is monitor-ready: stamps safety:reviewed + ctx/reasoning, no auto-implement" {
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"; mk_fill ok; mk_apply_safety "pass"
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote"]}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label groom:proposed' "$GH_LOG"
  grep -q 'add-label safety:reviewed' "$GH_LOG"
  grep -q 'add-label ctx:' "$GH_LOG"
  grep -q 'reasoning:' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
}

@test "final-body safety quarantine blocks promotion of an eligible candidate" {
  mk_safety "SAFETY_PASS"; mk_elig "eligible"; mk_apply_safety "quarantine"
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label security:quarantined' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
  printf '%s' "$output" | jq -e '.quarantined[] | select(.issue==42)' >/dev/null
}

@test "classify runs for a needs-template candidate with no ctx/reasoning labels" {
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"; mk_fill ok; mk_apply_safety "pass"
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote"]}'
  mk_view_labels '{"name":"needs-autospec-template"}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label ctx:' "$GH_LOG"
  grep -q 'reasoning:' "$GH_LOG"
  grep -q 'remove-label needs-classify' "$GH_LOG"
}

@test "final-body safety quarantine blocks promotion of a filled needs-template candidate" {
  # The higher-risk fill-generated path: a filled body that fails the final-body
  # safety stamp must be quarantined and never proposed/promoted.
  mk_safety "SAFETY_PASS"; mk_elig "needs-template"; mk_fill ok; mk_apply_safety "quarantine"
  export GROOM_GOVERN_ACTIVE='{"active":["eligible-promote"]}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  grep -q 'add-label security:quarantined' "$GH_LOG"
  ! grep -q 'add-label groom:proposed' "$GH_LOG"
  ! grep -q 'add-label auto-implement' "$GH_LOG"
  printf '%s' "$output" | jq -e '.quarantined[] | select(.issue==42)' >/dev/null
}
