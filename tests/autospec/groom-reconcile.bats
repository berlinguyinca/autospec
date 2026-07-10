#!/usr/bin/env bats

setup() {
  RECON_SH="${BATS_TEST_DIRNAME}/../../scripts/groom-reconcile.sh"   # tests/autospec/ → repo/scripts/
  BIN_DIR="$BATS_TEST_TMPDIR/bin"; mkdir -p "$BIN_DIR"
  TELE="$BATS_TEST_TMPDIR/tele.jsonl"
}
# a gh stub that returns a fixed json per issue number from files issue-<n>.json
mk_gh() {
  cat > "$BIN_DIR/gh" <<'SH'
#!/usr/bin/env bash
# args: issue view N --repo R --json ...
num=""
while [ $# -gt 0 ]; do case "$1" in view) num="$2"; shift 2;; *) shift;; esac; done
f="$GH_FIXTURE_DIR/issue-$num.json"
[ -f "$f" ] && cat "$f" || { echo '{}'; }
SH
  chmod +x "$BIN_DIR/gh"
}

@test "unresolved + closed-completed + merged PR → clean" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"CLOSED","stateReason":"COMPLETED","closedByPullRequestsReferences":[{"number":42}],"labels":[]}\n' > "$BATS_TEST_TMPDIR/fx/issue-10.json"
  printf '{"issue":10,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$status" -eq 0 ]
  [ "$(jq -r '.outcome' "$TELE")" = "clean" ]
  [ "$(jq -r '.closing_pr' "$TELE")" = "42" ]
}

@test "escalate:human label → escalate (even if closed clean-looking)" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"CLOSED","stateReason":"COMPLETED","closedByPullRequestsReferences":[{"number":9}],"labels":[{"name":"escalate:human"}]}\n' > "$BATS_TEST_TMPDIR/fx/issue-11.json"
  printf '{"issue":11,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$(jq -r '.outcome' "$TELE")" = "escalate" ]
}

@test "closed not-planned → rejected" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"CLOSED","stateReason":"NOT_PLANNED","closedByPullRequestsReferences":[],"labels":[]}\n' > "$BATS_TEST_TMPDIR/fx/issue-12.json"
  printf '{"issue":12,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$(jq -r '.outcome' "$TELE")" = "rejected" ]
}

@test "groom:rejected label → rejected" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"CLOSED","stateReason":"COMPLETED","closedByPullRequestsReferences":[{"number":5}],"labels":[{"name":"groom:rejected"}]}\n' > "$BATS_TEST_TMPDIR/fx/issue-13.json"
  printf '{"issue":13,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$(jq -r '.outcome' "$TELE")" = "rejected" ]
}

@test "still open → stays unresolved (null)" {
  mk_gh; mkdir -p "$BATS_TEST_TMPDIR/fx"
  printf '{"state":"OPEN","stateReason":null,"closedByPullRequestsReferences":[],"labels":[]}\n' > "$BATS_TEST_TMPDIR/fx/issue-14.json"
  printf '{"issue":14,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env GH_FIXTURE_DIR="$BATS_TEST_TMPDIR/fx" AUTOSPEC_GH_BIN="$BIN_DIR/gh" \
      bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$(jq -r '.outcome' "$TELE")" = "null" ]
}

@test "already-resolved record untouched (no gh call)" {
  # gh stub that fails loudly if called
  cat > "$BIN_DIR/gh-boom" <<'SH'
#!/usr/bin/env bash
echo "SHOULD NOT CALL" >&2; exit 99
SH
  chmod +x "$BIN_DIR/gh-boom"
  printf '{"issue":15,"template_groomed":true,"outcome":"clean","closing_pr":1}\n' > "$TELE"
  run env AUTOSPEC_GH_BIN="$BIN_DIR/gh-boom" bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$status" -eq 0 ]
  [ "$(jq -r '.outcome' "$TELE")" = "clean" ]
}

@test "gh failure leaves outcome null (fail-closed)" {
  cat > "$BIN_DIR/gh-fail" <<'SH'
#!/usr/bin/env bash
exit 1
SH
  chmod +x "$BIN_DIR/gh-fail"
  printf '{"issue":16,"template_groomed":true,"outcome":null}\n' > "$TELE"
  run env AUTOSPEC_GH_BIN="$BIN_DIR/gh-fail" bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$status" -eq 0 ]
  [ "$(jq -r '.outcome' "$TELE")" = "null" ]
}

@test "missing telemetry file → rc 0, no crash" {
  run env AUTOSPEC_GH_BIN="$BIN_DIR/gh" bash "$RECON_SH" --telemetry /nonexistent --repo o/r
  [ "$status" -eq 0 ]
}

@test "gh exits 0 but prints non-JSON garbage → record stays unresolved, sweep continues" {
  cat > "$BIN_DIR/gh-garbage" <<'SH'
#!/usr/bin/env bash
# args: issue view N --repo R --json ...
num=""
while [ $# -gt 0 ]; do case "$1" in view) num="$2"; shift 2;; *) shift;; esac; done
if [ "$num" = "20" ]; then
  echo "gh: unexpected notice text"
  exit 0
fi
printf '{"state":"CLOSED","stateReason":"COMPLETED","closedByPullRequestsReferences":[{"number":42}],"labels":[]}\n'
exit 0
SH
  chmod +x "$BIN_DIR/gh-garbage"
  printf '{"issue":20,"template_groomed":true,"outcome":null}\n' > "$TELE"
  printf '{"issue":21,"template_groomed":true,"outcome":null}\n' >> "$TELE"
  run env AUTOSPEC_GH_BIN="$BIN_DIR/gh-garbage" bash "$RECON_SH" --telemetry "$TELE" --repo o/r
  [ "$status" -eq 0 ]
  [ "$(jq -r 'select(.issue==20) | .outcome' "$TELE")" = "null" ]
  [ "$(jq -r 'select(.issue==21) | .outcome' "$TELE")" = "clean" ]
}
