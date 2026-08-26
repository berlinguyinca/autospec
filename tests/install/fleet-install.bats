#!/usr/bin/env bats
# fleet-run.sh must actually be installable and runnable on a clean install.
# It `source`s fleet-lib.sh and shells out to fleet-config-lint.sh, both
# resolved relative to its own installed directory — if either is missing
# from install.sh's registered set, the launcher hard-crashes on first use
# even though `skills/autospec-fleet/install.sh` reports success.
#
# The expected set is derived from fleet-run.sh's own `$script_dir/*.sh`
# references (source + bash invocations) rather than hardcoded, so this test
# does not silently rot if fleet-run.sh grows a new same-directory dependency.

setup() {
  REPO="${BATS_TEST_DIRNAME}/../.."
  INSTALL="$REPO/skills/autospec-fleet/install.sh"
  FLEET_RUN="$REPO/skills/autospec-fleet/scripts/fleet-run.sh"
}

# Pull a shell variable's value out of install.sh without sourcing it
# (sourcing would run the script's arg-parsing / harness-prompt logic).
read_var() {
  awk -F'"' -v name="$1" '$0 ~ "^"name"=\"" { print $2; exit }' "$INSTALL"
}

@test "every same-directory script fleet-run.sh sources or invokes is registered in FLEET_SCRIPT_FILES" {
  fleet_files="$(read_var FLEET_SCRIPT_FILES)"
  deps="$(grep -oE '\$script_dir/[a-zA-Z0-9_.-]+\.sh' "$FLEET_RUN" | sed 's#\$script_dir/##' | sort -u)"
  [ -n "$deps" ]
  for dep in $deps; do
    case " $fleet_files " in
      *" $dep "*) : ;;
      *) echo "unregistered: $dep"; false ;;
    esac
  done
}

@test "fleet-run.sh itself is registered in FLEET_SCRIPT_FILES" {
  fleet_files="$(read_var FLEET_SCRIPT_FILES)"
  case " $fleet_files " in
    *" fleet-run.sh "*) : ;;
    *) echo "unregistered: fleet-run.sh"; false ;;
  esac
}

@test "a clean isolated-HOME install lands the fleet scripts, executable, and fleet-run.sh runs from there" {
  TESTHOME="$(mktemp -d)"
  HOME="$TESTHOME" run sh "$INSTALL" --harness claude
  [ "$status" -eq 0 ]

  for f in fleet-run.sh fleet-lib.sh fleet-config-lint.sh; do
    [ -f "$TESTHOME/.autospec/scripts/$f" ]
    [ -x "$TESTHOME/.autospec/scripts/$f" ]
  done

  run bash "$TESTHOME/.autospec/scripts/fleet-run.sh" --help
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'Usage: fleet-run.sh'

  rm -rf "$TESTHOME"
}
