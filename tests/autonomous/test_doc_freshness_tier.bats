#!/usr/bin/env bats
# Unit tests for the documentation generation & freshness tier (#1540).

SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/doc-freshness-tier.sh"

setup() {
  WORK="$(mktemp -d)"
  mkdir -p "$WORK/bin" "$WORK/repo/docs" "$WORK/repo/.git"
  DIFF="$WORK/diff.txt"
}

teardown() {
  rm -rf "$WORK"
}

write_fake_drift() {
  local body="$1" rc="${2:-0}"
  cat > "$WORK/bin/check-doc-drift.sh" <<EOF2
#!/usr/bin/env bash
cat <<'JSON'
$body
JSON
exit $rc
EOF2
  chmod +x "$WORK/bin/check-doc-drift.sh"
}

@test "stale verified examples block the freshness gate" {
  write_fake_drift '{"passed":true,"drift":[],"missing_scope":[],"visual_stale":[],"example_stale":[{"doc_file":"docs/user/widget.md","heading":"Run it"}],"skipped":false}' 0
  run env AUTOSPEC_CHECK_DRIFT_SH="$WORK/bin/check-doc-drift.sh" bash "$SCRIPT" --working-tree --repo-root "$WORK/repo" --dry-run
  [ "$status" -eq 1 ]
  [[ "$output" == *"example_stale"* ]]
  [[ "$output" == *"docs/user/widget.md"* ]]
}

@test "public API/config drift proposes an auto-implement doc-update issue" {
  write_fake_drift '{"passed":false,"drift":[{"doc_file":"docs/user/api.md","heading":"Flags","matching_source_files":["scripts/new-flag.sh"],"reason":"flag changed"}],"missing_scope":[],"visual_stale":[],"example_stale":[],"skipped":false}' 1
  run env AUTOSPEC_CHECK_DRIFT_SH="$WORK/bin/check-doc-drift.sh" bash "$SCRIPT" --working-tree --repo-root "$WORK/repo" --dry-run
  [ "$status" -eq 1 ]
  [[ "$output" == *"doc-update issue proposal"* ]]
  [[ "$output" == *"auto-implement"* ]]
  [[ "$output" == *"scripts/new-flag.sh"* ]]
}

@test "doc-update issue filing stamps origin:self (issue #1785)" {
  write_fake_drift '{"passed":false,"drift":[{"doc_file":"docs/user/api.md","heading":"Flags","matching_source_files":["scripts/new-flag.sh"],"reason":"flag changed"}],"missing_scope":[],"visual_stale":[],"example_stale":[],"skipped":false}' 1
  cat > "$WORK/bin/gh" <<'EOF2'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"issue create"*) echo "https://github.com/example/repo/issues/1"; exit 0 ;;
esac
exit 0
EOF2
  chmod +x "$WORK/bin/gh"
  GH_LOG="$WORK/gh.log" run env PATH="$WORK/bin:$PATH" GH_LOG="$WORK/gh.log" AUTOSPEC_CHECK_DRIFT_SH="$WORK/bin/check-doc-drift.sh" bash "$SCRIPT" --working-tree --repo-root "$WORK/repo"
  grep -q 'label create origin:self' "$WORK/gh.log"
  tr '\n' ' ' < "$WORK/gh.log" | grep -qE 'issue create.*--label origin:self'
}

@test "changed docs run docs-as-tests and llms export regeneration" {
  write_fake_drift '{"passed":true,"drift":[],"missing_scope":[],"visual_stale":[],"example_stale":[],"skipped":false}' 0
  cat > "$WORK/bin/verify-examples.mjs" <<'EOF2'
#!/usr/bin/env node
import fs from 'node:fs';
fs.appendFileSync(process.env.CALL_LOG, `verify ${process.argv.slice(2).join(' ')}\n`);
EOF2
  chmod +x "$WORK/bin/verify-examples.mjs"
  cat > "$WORK/bin/gen-llms-txt.sh" <<'EOF2'
#!/usr/bin/env bash
printf 'gen %s\n' "$*" >> "$CALL_LOG"
EOF2
  chmod +x "$WORK/bin/gen-llms-txt.sh"
  mkdir -p "$WORK/repo/docs"
  printf '# User docs\n' > "$WORK/repo/docs/user.md"
  cat > "$DIFF" <<'EOF2'
diff --git a/docs/user.md b/docs/user.md
+++ b/docs/user.md
@@ -1 +1 @@
-# Old
+# User docs
EOF2
  CALL_LOG="$WORK/calls.log" run env AUTOSPEC_CHECK_DRIFT_SH="$WORK/bin/check-doc-drift.sh" AUTOSPEC_VERIFY_EXAMPLES_MJS="$WORK/bin/verify-examples.mjs" AUTOSPEC_GEN_LLMS_TXT_SH="$WORK/bin/gen-llms-txt.sh" CALL_LOG="$WORK/calls.log" bash "$SCRIPT" --diff "$DIFF" --repo-root "$WORK/repo" --dry-run
  [ "$status" -eq 0 ]
  [[ "$(cat "$WORK/calls.log")" == *"verify $WORK/repo/docs/user.md"* ]]
  [[ "$(cat "$WORK/calls.log")" == *"gen --repo-root $WORK/repo"* ]]
}

@test "docs-as-tests verifier failure blocks the freshness gate" {
  write_fake_drift '{"passed":true,"drift":[],"missing_scope":[],"visual_stale":[],"example_stale":[],"skipped":false}' 0
  cat > "$WORK/bin/verify-examples.mjs" <<'EOF2'
#!/usr/bin/env node
process.exit(9);
EOF2
  chmod +x "$WORK/bin/verify-examples.mjs"
  cat > "$WORK/bin/gen-llms-txt.sh" <<'EOF2'
#!/usr/bin/env bash
printf 'gen %s\n' "$*" >> "$CALL_LOG"
EOF2
  chmod +x "$WORK/bin/gen-llms-txt.sh"
  printf '# User docs\n' > "$WORK/repo/docs/user.md"
  cat > "$DIFF" <<'EOF2'
diff --git a/docs/user.md b/docs/user.md
+++ b/docs/user.md
@@ -1 +1 @@
-# Old
+# User docs
EOF2
  CALL_LOG="$WORK/calls.log" run env AUTOSPEC_CHECK_DRIFT_SH="$WORK/bin/check-doc-drift.sh" AUTOSPEC_VERIFY_EXAMPLES_MJS="$WORK/bin/verify-examples.mjs" AUTOSPEC_GEN_LLMS_TXT_SH="$WORK/bin/gen-llms-txt.sh" CALL_LOG="$WORK/calls.log" bash "$SCRIPT" --diff "$DIFF" --repo-root "$WORK/repo" --dry-run
  [ "$status" -eq 1 ]
  [[ "$output" == *"docs-as-tests example verification failed"* ]]
  [ ! -e "$WORK/calls.log" ]
}
