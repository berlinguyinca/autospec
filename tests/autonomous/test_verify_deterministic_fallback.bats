#!/usr/bin/env bats

DRAIN="$BATS_TEST_DIRNAME/../../scripts/autospec-autonomous-verify-drain.sh"

@test "verifier exposes deterministic fallback for bounded evidence" {
  grep -q 'deterministic_fallback' "$DRAIN"
  grep -q 'AUTOSPEC_AUTONOMOUS_DETERMINISTIC_VERIFY' "$DRAIN"
}

@test "verifier fallback requires an existing path and line" {
  grep -q 'os.path.isfile(path)' "$DRAIN"
  grep -q 're.search' "$DRAIN"
}

@test "verifier watchdog delegates to the shared process-tree helper" {
  grep -q 'lib/autospec-process-tree.sh' "$DRAIN"
  grep -q 'autospec_kill_tree "\$child_pid" separate-recursive' "$DRAIN"
  # The drain delegates to the shared lib; no local kill-tree definition remains.
  ! grep -qE '^[[:space:]]*kill_tree[[:space:]]*\(\)' "$DRAIN"
}

@test "verifier watchdog leaves 0 verifier-owned descendants alive" {
  TMP="$(mktemp -d -t verify-fallback-liveness.XXXXXX)"
  mkdir -p "$TMP/bin"
  export PATH="$TMP/bin:$PATH"
  DEDUPED_IN="$TMP/dedup.json"
  VERDICTS_OUT="$TMP/verdicts.json"
  DESC_FILE="$TMP/descendant.pid"
  export AUTOSPEC_EXPLORE_DEDUPED_IN="$DEDUPED_IN"
  export AUTOSPEC_EXPLORE_VERDICTS_OUT="$VERDICTS_OUT"
  export AUTOSPEC_AUTONOMOUS_VERIFY_STALL_SECS=2
  export AUTOSPEC_AUTONOMOUS_VERIFY_POLL_SECS=1
  export AUTOSPEC_REPO_DIR="$TMP"

  # Evidence names no real repository path, so the deterministic fallback
  # rejects and the drain fails closed after the watchdog terminates omx.
  cat > "$DEDUPED_IN" <<'JSON'
{"deduped":[
  {"norm_title":"stall probe","title":"feat: stall probe","evidence":"no repository path here","estimated_complexity":"small","confidence":0.5}
]}
JSON

  # The omx shim spawns a long-lived descendant inside its isolated process
  # group, then goes silent so the stall watchdog fires.
  cat > "$TMP/bin/omx" <<EOF
#!/usr/bin/env bash
sleep 300 &
echo "\$!" > "$DESC_FILE"
printf 'skeptic started\n'
sleep 300
EOF
  chmod +x "$TMP/bin/omx"

  run bash "$DRAIN"
  [ "$status" -ne 0 ] || { echo "$output"; false; }

  desc_pid="$(cat "$DESC_FILE" 2>/dev/null || printf '')"
  [ -n "$desc_pid" ]
  for _ in 1 2 3 4 5 6 7 8 9 10; do
      kill -0 "$desc_pid" 2>/dev/null || break
      sleep 0.5
  done
  ! kill -0 "$desc_pid" 2>/dev/null

  rm -rf "$TMP"
}
