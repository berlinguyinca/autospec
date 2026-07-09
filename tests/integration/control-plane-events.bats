#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

@test "integration wrapper verifies generated event ingestion contract" {
  run bats --print-output-on-failure "${BATS_TEST_DIRNAME}/../control-plane-events.bats"
  if [ "$status" -ne 0 ]; then
    printf '%s\n' "$output" >&2
    false
  fi
}
