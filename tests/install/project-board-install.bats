#!/usr/bin/env bats
# The board scripts, and the Tier 1.5 grooming dependency set they run
# inside of, must ship on a clean install. An unregistered runtime script is
# silently absent after `install.sh` and hard-crashes the conductor's
# Tier 1.5 board stage the first time it is exercised.
#
# The expected sets are derived from real sources (scripts/project-board-*.sh
# on disk, and the *_SCRIPT seam variables in
# scripts/autonomous-promote-open-issues.sh) rather than hardcoded, so this
# test does not silently rot if the dependency set changes.

setup() {
  REPO="${BATS_TEST_DIRNAME}/../.."
  INSTALL="$REPO/skills/autospec-autonomous/install.sh"
  PROMOTER="$REPO/scripts/autonomous-promote-open-issues.sh"
}

# Pull a shell array variable's value out of install.sh without sourcing it
# (sourcing would run the script's arg-parsing / harness-prompt logic).
read_var() {
  awk -F'"' -v name="$1" '$0 ~ "^"name"=\"" { print $2; exit }' "$INSTALL"
}

@test "every project-board-*.sh script on disk exists and is executable" {
  found=0
  for f in "$REPO"/scripts/project-board-*.sh; do
    [ -f "$f" ]
    [ -x "$f" ]
    found=$((found + 1))
  done
  [ "$found" -ge 4 ]
}

@test "every project-board-*.sh script on disk is registered in AUTONOMOUS_SCRIPT_FILES" {
  autonomous_files="$(read_var AUTONOMOUS_SCRIPT_FILES)"
  for f in "$REPO"/scripts/project-board-*.sh; do
    name="$(basename "$f")"
    case " $autonomous_files " in
      *" $name "*) : ;;
      *) echo "unregistered: $name"; false ;;
    esac
  done
}

@test "the Tier 1.5 promoter and its repo-root grooming dependencies are registered in AUTONOMOUS_SCRIPT_FILES" {
  autonomous_files="$(read_var AUTONOMOUS_SCRIPT_FILES)"
  case " $autonomous_files " in
    *" autonomous-promote-open-issues.sh "*) : ;;
    *) echo "unregistered: autonomous-promote-open-issues.sh"; false ;;
  esac
  # Derive the repo-root-scripts seam deps from the promoter's own
  # `${AUTOSPEC_..._SCRIPT:-$SCRIPT_DIR/<name>.sh}` seam declarations.
  deps="$(grep -oE '\$SCRIPT_DIR/[a-zA-Z0-9_.-]+\.sh' "$PROMOTER" | sed 's#\$SCRIPT_DIR/##' | sort -u)"
  [ -n "$deps" ]
  for dep in $deps; do
    case " $autonomous_files " in
      *" $dep "*) : ;;
      *) echo "unregistered repo-root grooming dependency: $dep"; false ;;
    esac
  done
}

@test "the promoter's shared-lib dependency (grooming-config.sh) is registered in SHARED_LIB_SCRIPT_FILES, not AUTONOMOUS_SCRIPT_FILES" {
  autonomous_files="$(read_var AUTONOMOUS_SCRIPT_FILES)"
  shared_lib_files="$(read_var SHARED_LIB_SCRIPT_FILES)"

  deps="$(grep -oE '\$SHARED_DIR/[a-zA-Z0-9_.-]+\.sh' "$PROMOTER" | sed 's#\$SHARED_DIR/##' | sort -u)"
  [ -n "$deps" ]
  for dep in $deps; do
    case " $shared_lib_files " in
      *" $dep "*) : ;;
      *) echo "missing from SHARED_LIB_SCRIPT_FILES: $dep"; false ;;
    esac
    # A shared-lib dependency must not also be miscategorized into the
    # repo-root group — that was a known past install.sh bug.
    case " $autonomous_files " in
      *" $dep "*) echo "shared-lib dep wrongly also in AUTONOMOUS_SCRIPT_FILES: $dep"; false ;;
      *) : ;;
    esac
    [ -f "$REPO/skills/autospec-shared/scripts/$dep" ]
  done
}

@test "install.sh installs shared-lib scripts from skills/autospec-shared/scripts, not repo-root scripts/" {
  grep -q 'resolve_shared_lib_scripts_dir' "$INSTALL"
  grep -q 'skills/autospec-shared/scripts' "$INSTALL"
}

@test "list-groomable.sh is registered (candidate-set feeder for the promoter)" {
  autonomous_files="$(read_var AUTONOMOUS_SCRIPT_FILES)"
  case " $autonomous_files " in
    *" list-groomable.sh "*) : ;;
    *) echo "unregistered: list-groomable.sh"; false ;;
  esac
}

@test "the control mirror script is registered" {
  grep -q 'project-board-control-mirror.sh' "$INSTALL"
  [ -x "$REPO/scripts/project-board-control-mirror.sh" ]
}

@test "clean installs verify the typed managed-project command before installing workflow skills" {
  grep -q '^verify_managed_project_command_surface()' "$REPO/install.sh"
  verify_line="$(grep -n '^verify_managed_project_command_surface$' "$REPO/install.sh" | tail -1 | cut -d: -f1)"
  loop_line="$(grep -n '^for skill in \$SKILLS_TO_RUN; do$' "$REPO/install.sh" | cut -d: -f1)"
  [ -n "$verify_line" ]
  [ -n "$loop_line" ]
  [ "$verify_line" -lt "$loop_line" ]
  grep -q '^for dep in autospec git gh jq; do$' "$REPO/skills/autospec-project/install.sh"
}

@test "standalone project installer fails closed when runtime installation cannot expose project modes" {
  test_root="$(mktemp -d)"
  mkdir -p "$test_root/bin" "$test_root/home"
  cat > "$test_root/bin/autospec" <<'SH'
#!/usr/bin/env sh
exit 1
SH
  chmod +x "$test_root/bin/autospec"

  run env HOME="$test_root/home" PATH="$test_root/bin:$PATH" \
    AUTOSPEC_BIN="$test_root/bin/autospec" AUTOSPEC_PROJECT_RUNTIME_INSTALLER=/bin/false \
    sh "$REPO/skills/autospec-project/install.sh" --harness codex

  [ "$status" -ne 0 ]
  [[ "$output" == *"autospec project modes are unavailable"* ]]
  [[ "$output" != *"Installed autospec-project."* ]]
  rm -rf "$test_root"
}

@test "standalone project installer accepts an established installer that exposes project modes" {
  test_root="$(mktemp -d)"
  mkdir -p "$test_root/bin" "$test_root/home"
  cat > "$test_root/bin/autospec" <<'SH'
#!/usr/bin/env sh
exit 1
SH
  cat > "$test_root/runtime-install" <<'SH'
#!/usr/bin/env sh
runtime="$HOME/.autospec/bin/autospec"
mkdir -p "$(dirname "$runtime")"
cat > "$runtime" <<'RUNTIME'
#!/usr/bin/env sh
case "$*" in
  "project onboard --help") printf '%s\n' 'autospec project onboard --repo-dir PATH --spawned-from IDENTITY' ;;
  "project sync --help") printf '%s\n' 'autospec project sync --repo-dir PATH' ;;
  *) exit 1 ;;
esac
RUNTIME
chmod +x "$runtime"
printf '%s\n' "$runtime"
SH
  chmod +x "$test_root/bin/autospec" "$test_root/runtime-install"

  run env HOME="$test_root/home" PATH="$test_root/bin:$PATH" \
    AUTOSPEC_BIN="$test_root/bin/autospec" AUTOSPEC_PROJECT_RUNTIME_INSTALLER="$test_root/runtime-install" \
    sh "$REPO/skills/autospec-project/install.sh" --harness codex

  [ "$status" -eq 0 ]
  [[ "$output" == *"Installed autospec-project."* ]]
  "$test_root/home/.autospec/bin/autospec" project onboard --help | grep -F -- '--spawned-from'
  "$test_root/home/.autospec/bin/autospec" project sync --help | grep -F 'autospec project sync'
  rm -rf "$test_root"
}

@test "standalone project installer rejects a generic project help surface without Task 6 modes" {
  test_root="$(mktemp -d)"
  mkdir -p "$test_root/bin" "$test_root/home"
  cat > "$test_root/bin/autospec" <<'SH'
#!/usr/bin/env sh
[ "$*" = "project --help" ] && printf '%s\n' 'autospec project'
SH
  chmod +x "$test_root/bin/autospec"

  run env HOME="$test_root/home" PATH="$test_root/bin:$PATH" \
    AUTOSPEC_BIN="$test_root/bin/autospec" AUTOSPEC_PROJECT_RUNTIME_INSTALLER=/bin/false \
    sh "$REPO/skills/autospec-project/install.sh" --harness codex

  [ "$status" -ne 0 ]
  [[ "$output" == *"autospec project modes are unavailable"* ]]
  [[ "$output" != *"Installed autospec-project."* ]]
  rm -rf "$test_root"
}
