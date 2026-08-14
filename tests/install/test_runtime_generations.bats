#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_ROOT="$(mktemp -d "${BATS_TMPDIR:-/tmp}/runtime-generations.XXXXXX")"
  TEST_HOME="$TEST_ROOT/home"
  FIXTURE_REPO="$TEST_ROOT/repo"
  FAKE_BIN="$TEST_ROOT/bin"
  BUILD_LOG="$TEST_ROOT/build.log"
  mkdir -p "$TEST_HOME" "$FIXTURE_REPO/crates/demo/src" \
    "$FIXTURE_REPO/crates/autospec-cli" "$FAKE_BIN"
  printf '[workspace]\nmembers=["crates/demo"]\n' >"$FIXTURE_REPO/Cargo.toml"
  printf '# lock\n' >"$FIXTURE_REPO/Cargo.lock"
  printf '[package]\nname="demo"\nversion="0.1.0"\n' >"$FIXTURE_REPO/crates/demo/Cargo.toml"
  printf '[package]\nname="autospec-cli"\nversion="0.1.0"\n' \
    >"$FIXTURE_REPO/crates/autospec-cli/Cargo.toml"
  printf 'pub const BUILD: &str = "one";\n' >"$FIXTURE_REPO/crates/demo/src/lib.rs"
  git -C "$FIXTURE_REPO" init -q
  git -C "$FIXTURE_REPO" config user.email test@example.com
  git -C "$FIXTURE_REPO" config user.name 'Generation Test'
  git -C "$FIXTURE_REPO" add .
  git -C "$FIXTURE_REPO" commit -qm fixture

  cat >"$FAKE_BIN/cargo" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'build %s\n' "$PWD" >>"$AUTOSPEC_TEST_BUILD_LOG"
mkdir -p "$CARGO_TARGET_DIR/release"
cp "$PWD/crates/demo/src/lib.rs" "$CARGO_TARGET_DIR/release/autospec"
chmod +x "$CARGO_TARGET_DIR/release/autospec"
if [ -n "${AUTOSPEC_TEST_BUILD_ENTERED:-}" ]; then
  : >"$AUTOSPEC_TEST_BUILD_ENTERED"
  while [ ! -e "$AUTOSPEC_TEST_BUILD_RELEASE" ]; do sleep 0.02; done
fi
if [ "${AUTOSPEC_TEST_MUTATE_SOURCE:-0}" = 1 ]; then
  printf '// moved\n' >>"$PWD/crates/demo/src/lib.rs"
fi
SH
  chmod +x "$FAKE_BIN/cargo"
}

@test "runtime source resolution keeps a generic target checkout separate" {
  install_runtime
  [ "$status" -eq 0 ]
  target="$TEST_ROOT/target"
  mkdir -p "$target"
  git -C "$target" init -q

  run env HOME="$TEST_HOME" bash "$REPO_ROOT/scripts/autonomous-runtime-refresh.sh" \
    source --repo-dir "$target"

  [ "$status" -eq 0 ]
  [ "$output" = "$(cd -P "$FIXTURE_REPO" && pwd -P)" ]
}

teardown() {
  find "$TEST_ROOT" -type d -exec chmod u+w {} + 2>/dev/null || true
  rm -rf "$TEST_ROOT"
}

install_runtime() {
  run env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO"
}

mode_of() {
  stat -f '%Lp' "$1" 2>/dev/null || stat -c '%a' "$1"
}

wait_for_file() {
  file=$1
  waiting_pid=$2
  attempts=0
  while [ ! -e "$file" ] && kill -0 "$waiting_pid" 2>/dev/null && [ "$attempts" -lt 250 ]; do
    sleep 0.02
    attempts=$((attempts + 1))
  done
  [ -e "$file" ]
}

@test "installer publishes an immutable digest generation and prints its exact executable" {
  install_runtime
  [ "$status" -eq 0 ]
  generation_binary="$output"
  [ -x "$generation_binary" ]
  digest="$(basename "$(dirname "$generation_binary")")"
  [[ "$digest" =~ ^[0-9a-f]{64}$ ]]
  [ "$generation_binary" = "$TEST_HOME/.autospec/runtime-generations/$digest/autospec" ]
  [ -f "$(dirname "$generation_binary")/receipt" ]
  [ "$(mode_of "$TEST_HOME/.autospec")" = 700 ]
  [ "$(mode_of "$(dirname "$generation_binary")")" = 500 ]
  [ "$(mode_of "$generation_binary")" = 500 ]
  [ "$(mode_of "$(dirname "$generation_binary")/receipt")" = 400 ]
  [ "$(readlink "$TEST_HOME/.autospec/runtime-generations/current")" = "$digest" ]

  before="$(shasum -a 256 "$generation_binary" | awk '{print $1}')"
  install_runtime
  [ "$status" -eq 0 ]
  [ "$output" = "$generation_binary" ]
  [ "$(wc -l <"$BUILD_LOG" | tr -d ' ')" -eq 1 ]
  [ "$(shasum -a 256 "$generation_binary" | awk '{print $1}')" = "$before" ]
}

@test "warm generation reuse avoids a second source inventory and stays bounded" {
  install_runtime
  [ "$status" -eq 0 ]
  real_git="$(command -v git)"
  cat >"$FAKE_BIN/git" <<SH
#!/usr/bin/env bash
for arg in "\$@"; do
  if [ "\$arg" = ls-files ]; then printf 'inventory\n' >>'$TEST_ROOT/inventory.log'; fi
done
exec '$real_git' "\$@"
SH
  chmod +x "$FAKE_BIN/git"
  { /usr/bin/time -p env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/warm.path"; } 2>"$TEST_ROOT/warm.time"
  warm_status=$?
  elapsed_seconds="$(awk '/^real / { print $2 }' "$TEST_ROOT/warm.time")"
  echo "warm ensure elapsed: ${elapsed_seconds}s" >&3
  [ "$warm_status" -eq 0 ]
  [ "$(cat "$TEST_ROOT/warm.path")" = "$output" ]
  [ ! -e "$TEST_ROOT/inventory.log" ]
  awk -v elapsed="$elapsed_seconds" 'BEGIN { exit !(elapsed <= 0.20) }'
}

@test "a generation built from a dirty snapshot is never reused after the checkout becomes clean" {
  printf '// dirty build\n' >>"$FIXTURE_REPO/crates/demo/src/lib.rs"
  install_runtime
  [ "$status" -eq 0 ]
  dirty_path="$output"
  git -C "$FIXTURE_REPO" restore crates/demo/src/lib.rs
  install_runtime
  [ "$status" -eq 0 ]
  [ "$output" != "$dirty_path" ]
  [ "$(wc -l <"$BUILD_LOG" | tr -d ' ')" -eq 2 ]
}

@test "installer rejects a moving source and retains the prior current generation" {
  install_runtime
  [ "$status" -eq 0 ]
  prior="$output"
  printf 'pub const BUILD: &str = "two";\n' >"$FIXTURE_REPO/crates/demo/src/lib.rs"

  run env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    AUTOSPEC_TEST_MUTATE_SOURCE=1 bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO"
  [ "$status" -ne 0 ]
  [ "$(readlink "$TEST_HOME/.autospec/runtime-generations/current")" = "$(basename "$(dirname "$prior")")" ]
  [ -x "$prior" ]
}

@test "concurrent repositories each receive the exact generation they built" {
  OTHER_REPO="$TEST_ROOT/other"
  cp -R "$FIXTURE_REPO" "$OTHER_REPO"
  git -C "$OTHER_REPO" config user.email test@example.com
  printf 'pub const BUILD: &str = "other";\n' >"$OTHER_REPO/crates/demo/src/lib.rs"

  env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/one.path" &
  one_pid=$!
  env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$OTHER_REPO" >"$TEST_ROOT/two.path" &
  two_pid=$!
  wait "$one_pid"
  wait "$two_pid"

  one_path="$(cat "$TEST_ROOT/one.path")"
  two_path="$(cat "$TEST_ROOT/two.path")"
  [ "$one_path" != "$two_path" ]
  grep -q 'one' "$one_path"
  grep -q 'other' "$two_path"
}

@test "lock disappearance during concurrent publication retries instead of reporting unsafe lock" {
  for attempt in 1 2 3 4 5 6; do
    attempt_home="$TEST_ROOT/race-home-$attempt"
    mkdir "$attempt_home"
    env HOME="$attempt_home" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
      bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/race-a-$attempt" & a=$!
    env HOME="$attempt_home" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
      bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/race-b-$attempt" & b=$!
    wait "$a"
    wait "$b"
    [ -x "$(cat "$TEST_ROOT/race-a-$attempt")" ]
    [ "$(cat "$TEST_ROOT/race-a-$attempt")" = "$(cat "$TEST_ROOT/race-b-$attempt")" ]
  done
}

@test "same-size same-mtime tracked source spoof cannot reuse the warm binary" {
  install_runtime
  [ "$status" -eq 0 ]
  prior="$output"
  source="$FIXTURE_REPO/crates/demo/src/lib.rs"
  python3 - "$source" <<'PY'
import os, sys
path = sys.argv[1]; before = os.stat(path)
data = open(path, encoding="utf-8").read()
replacement = data.replace('"one"', '"two"')
assert len(replacement) == len(data)
open(path, "w", encoding="utf-8").write(replacement)
os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
PY
  install_runtime
  [ "$status" -eq 0 ]
  [ "$output" != "$prior" ]
  grep -q 'two' "$output"
}

@test "signal cleanup and SIGKILL recovery never expose a partial generation" {
  entered="$TEST_ROOT/entered"
  release="$TEST_ROOT/release"
  env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    AUTOSPEC_TEST_BUILD_ENTERED="$entered" AUTOSPEC_TEST_BUILD_RELEASE="$release" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/path" &
  pid=$!
  wait_for_file "$entered" "$pid"
  kill -TERM "$pid"
  touch "$release"
  wait "$pid" || true
  [ ! -e "$TEST_HOME/.autospec/runtime-install.lock" ]
  [ ! -e "$TEST_HOME/.autospec/runtime-install.transaction" ]
  [ ! -e "$TEST_HOME/.autospec/runtime-generations/current" ]

  rm -f "$entered" "$release"
  env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    AUTOSPEC_TEST_BUILD_ENTERED="$entered" AUTOSPEC_TEST_BUILD_RELEASE="$release" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/path" &
  pid=$!
  wait_for_file "$entered" "$pid"
  kill -KILL "$pid"
  touch "$release"
  wait "$pid" || true

  install_runtime
  [ "$status" -eq 0 ]
  [ -x "$output" ]
  [ ! -e "$TEST_HOME/.autospec/runtime-install.lock" ]
  [ ! -e "$TEST_HOME/.autospec/runtime-install.transaction" ]
}

@test "crashes immediately before and after generation rename recover through the durable journal" {
  real_mv="$(command -v mv)"
  for boundary in before after; do
    boundary_home="$TEST_ROOT/home-$boundary"
    mkdir "$boundary_home"
    cat >"$FAKE_BIN/mv" <<SH
#!/usr/bin/env bash
case "\$1" in */.stage.*)
  if [ '$boundary' = before ]; then kill -KILL "\$PPID"; sleep 1; exit 137; fi
  '$real_mv' "\$@" || exit \$?
  kill -KILL "\$PPID"; sleep 1; exit 137 ;;
esac
exec '$real_mv' "\$@"
SH
    chmod +x "$FAKE_BIN/mv"
    env HOME="$boundary_home" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
      bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/$boundary.path" &
    pid=$!
    wait "$pid" || true
    [ -f "$boundary_home/.autospec/runtime-install.transaction" ]
    rm "$FAKE_BIN/mv"
    run env HOME="$boundary_home" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
      bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO"
    [ "$status" -eq 0 ]
    [ -x "$output" ]
    [ ! -e "$boundary_home/.autospec/runtime-install.transaction" ]
  done
}

@test "planned journal survives crashes at stage and build directory creation boundaries" {
  real_mkdir="$(command -v mkdir)"
  for boundary in .stage. .runtime-build.; do
    boundary_home="$TEST_ROOT/mkdir-${boundary//./x}"
    mkdir "$boundary_home"
    cat >"$FAKE_BIN/mkdir" <<SH
#!/usr/bin/env bash
case "\$*" in *'$boundary'*) kill -KILL "\$PPID"; sleep 1; exit 137 ;; esac
exec '$real_mkdir' "\$@"
SH
    chmod +x "$FAKE_BIN/mkdir"
    env HOME="$boundary_home" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
      bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >/dev/null &
    pid=$!
    wait "$pid" || true
    journal="$boundary_home/.autospec/runtime-install.transaction"
    [ -f "$journal" ]
    grep -q '^phase=planned$' "$journal"
    rm "$FAKE_BIN/mkdir"
    run env HOME="$boundary_home" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
      bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO"
    [ "$status" -eq 0 ]
  done
}

@test "planned journal from a moved checkout recovers without bricking global installation" {
  mkdir -p "$TEST_HOME/.autospec/runtime-generations"
  chmod 700 "$TEST_HOME/.autospec" "$TEST_HOME/.autospec/runtime-generations"
  source_sha="$(bash -c 'source "$1"; autospec_runtime_source_digest "$2"' _ "$REPO_ROOT/scripts/autonomous-runtime-refresh.sh" "$FIXTURE_REPO")"
  head="$(git -C "$FIXTURE_REPO" rev-parse HEAD)"
  digest="$(bash "$REPO_ROOT/scripts/autonomous-runtime-refresh.sh" identity --repo-dir "$FIXTURE_REPO")"
  old_repo="$FIXTURE_REPO"
  moved_repo="$TEST_ROOT/moved-repo"
  journal="$TEST_HOME/.autospec/runtime-install.transaction"
  printf 'schema=1\nphase=planned\nrepo=%s\nhead=%s\nsource_sha256=%s\ndigest=%s\nstage=%s\nbuild=%s\ndestination=%s\n' \
    "$old_repo" "$head" "$source_sha" "$digest" \
    "$TEST_HOME/.autospec/runtime-generations/.stage.$digest" "$TEST_HOME/.autospec/.runtime-build.$digest" \
    "$TEST_HOME/.autospec/runtime-generations/$digest" >"$journal"
  chmod 600 "$journal"
  mv "$FIXTURE_REPO" "$moved_repo"
  FIXTURE_REPO="$moved_repo"
  install_runtime
  [ "$status" -eq 0 ]
  [ -x "$output" ]
  [ ! -e "$journal" ]
}

@test "launcher check stays within the 200ms correctness-first warm target" {
  install_runtime
  [ "$status" -eq 0 ]
  real_git="$(command -v git)"
  cat >"$FAKE_BIN/git" <<SH
#!/usr/bin/env bash
for arg in "\$@"; do [ "\$arg" = ls-files ] && printf 'inventory\n' >>'$TEST_ROOT/check-inventory'; done
exec '$real_git' "\$@"
SH
  chmod +x "$FAKE_BIN/git"
  { /usr/bin/time -p env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" \
    bash "$REPO_ROOT/scripts/autonomous-runtime-refresh.sh" check --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/check.path"; } 2>"$TEST_ROOT/check.time"
  elapsed="$(awk '/^real / { print $2 }' "$TEST_ROOT/check.time")"
  echo "launcher check elapsed: ${elapsed}s" >&3
  [ "$(cat "$TEST_ROOT/check.path")" = "$output" ]
  [ ! -e "$TEST_ROOT/check-inventory" ]
  awk -v elapsed="$elapsed" 'BEGIN { exit !(elapsed <= 0.20) }'
}

@test "batched warm snapshot remains bounded across a representative large tree" {
  asset_root="$FIXTURE_REPO/crates/demo/generated-assets"
  mkdir -p "$asset_root"
  index=1
  while [ "$index" -le 1500 ]; do
    printf 'asset-%s\n' "$index" >"$asset_root/asset-$index.txt"
    index=$((index + 1))
  done
  git -C "$FIXTURE_REPO" add crates/demo/generated-assets
  git -C "$FIXTURE_REPO" commit -qm 'large asset fixture'
  install_runtime
  [ "$status" -eq 0 ]
  expected="$output"
  { /usr/bin/time -p env HOME="$TEST_HOME" PATH="$FAKE_BIN:$PATH" \
    bash "$REPO_ROOT/scripts/autonomous-runtime-refresh.sh" check --repo-dir "$FIXTURE_REPO" >"$TEST_ROOT/large-check.path"; } 2>"$TEST_ROOT/large-check.time"
  elapsed="$(awk '/^real / { print $2 }' "$TEST_ROOT/large-check.time")"
  echo "large-tree launcher check elapsed: ${elapsed}s" >&3
  [ "$(cat "$TEST_ROOT/large-check.path")" = "$expected" ]
  awk -v elapsed="$elapsed" 'BEGIN { exit !(elapsed <= 0.20) }'
}

@test "partial lock acquisition is never exposed and stale ownerless legacy locks recover" {
  lock="$TEST_HOME/.autospec/runtime-install.lock"
  mkdir -p "$lock"
  chmod 700 "$TEST_HOME/.autospec" "$lock"
  install_runtime
  [ "$status" -eq 0 ]
  [ -x "$output" ]

  other_home="$TEST_ROOT/lock-crash-home"
  mkdir "$other_home"
  real_mv="$(command -v mv)"
  cat >"$FAKE_BIN/mv" <<SH
#!/usr/bin/env bash
case "\$*" in *runtime-install.lock*) kill -KILL "\$PPID"; sleep 1; exit 137 ;; esac
exec '$real_mv' "\$@"
SH
  chmod +x "$FAKE_BIN/mv"
  env HOME="$other_home" PATH="$FAKE_BIN:$PATH" AUTOSPEC_TEST_BUILD_LOG="$BUILD_LOG" \
    bash "$REPO_ROOT/scripts/autospec-runtime-install.sh" --repo-dir "$FIXTURE_REPO" >/dev/null &
  pid=$!
  wait "$pid" || true
  [ ! -e "$other_home/.autospec/runtime-install.lock" ]
}

@test "lock metadata rejects ambiguity but reclaims a reused process identity" {
  lock="$TEST_HOME/.autospec/runtime-install.lock"
  mkdir -p "$lock"
  chmod 700 "$TEST_HOME/.autospec" "$lock"
  printf 'not metadata\n' >"$lock/owner"
  chmod 600 "$lock/owner"
  install_runtime
  [ "$status" -ne 0 ]

  rm -rf "$lock"
  mkdir "$lock"
  chmod 700 "$lock"
  printf 'pid=%s\nstart=definitely-not-this-process\ncreated_at=2026-08-13T00:00:00Z\n' "$$" >"$lock/owner"
  chmod 600 "$lock/owner"
  install_runtime
  [ "$status" -eq 0 ]
  [ -x "$output" ]
}

@test "lock metadata accepts only canonical positive platform PIDs" {
  lock="$TEST_HOME/.autospec/runtime-install.lock"
  if [ -r /proc/sys/kernel/pid_max ]; then pid_max="$(cat /proc/sys/kernel/pid_max)"; else pid_max="$(sysctl -n kern.pid_max 2>/dev/null || printf 99999)"; fi
  over_max=$((pid_max + 1))
  for pid in 0 -1 +1 01 "$over_max" 999999999999999999999999; do
    rm -rf "$lock"
    mkdir -p "$lock"
    chmod 700 "$TEST_HOME/.autospec" "$lock"
    printf 'pid=%s\nstart=x\ncreated_at=2026-08-13T00:00:00Z\n' "$pid" >"$lock/owner"
    chmod 600 "$lock/owner"
    install_runtime
    [ "$status" -ne 0 ]
  done
}

@test "malformed transaction paths fail closed and preserve recovery evidence" {
  mkdir -p "$TEST_HOME/.autospec/runtime-generations"
  chmod 700 "$TEST_HOME/.autospec" "$TEST_HOME/.autospec/runtime-generations"
  journal="$TEST_HOME/.autospec/runtime-install.transaction"
  printf 'schema=1\nphase=sealed\nrepo=%s\nhead=%040d\nsource_sha256=%064d\ndigest=%064d\nstage=%s\nbuild=%s\ndestination=%s\n' \
    "$FIXTURE_REPO" 0 0 0 "$TEST_HOME/.autospec/runtime-generations/../escape" \
    "$TEST_HOME/.autospec/.runtime-build.bad" "$TEST_HOME/.autospec/runtime-generations/bad" >"$journal"
  chmod 600 "$journal"
  install_runtime
  [ "$status" -ne 0 ]
  [ -f "$journal" ]
}

@test "pointer publication uses a temporary symlink and directory sync" {
  real_python="$(command -v python3)"
  cat >"$FAKE_BIN/python3" <<SH
#!/usr/bin/env bash
script='$TEST_ROOT/python-script'
cat >"\$script"
grep -E 'os\.replace|os\.fsync' "\$script" >>'$TEST_ROOT/python.log' || true
exec '$real_python' "\$@" <"\$script"
SH
  chmod +x "$FAKE_BIN/python3"
  install_runtime
  [ "$status" -eq 0 ]
  grep -q 'replace' "$TEST_ROOT/python.log"
  grep -q 'fsync' "$TEST_ROOT/python.log"
  [ -L "$TEST_HOME/.autospec/runtime-generations/current" ]
}

@test "top-level installer delegates runtime publication to the narrow installer" {
  run env HOME="$TEST_HOME" AUTOSPEC_SKIP_SYSTEM_TOOLS=1 AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_SUPERPOWERS=1 AUTOSPEC_SKIP_OH_MY_CODEX=1 AUTOSPEC_SKIP_OH_MY_OPENCODE=1 \
    AUTOSPEC_SKIP_OH_MY_CLAUDE=1 AUTOSPEC_NO_STAR_PROMPT=1 CI=1 \
    bash "$REPO_ROOT/install.sh" --skill autospec-autonomous --harness codex --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-runtime-install.sh --repo-dir"* ]]
}
