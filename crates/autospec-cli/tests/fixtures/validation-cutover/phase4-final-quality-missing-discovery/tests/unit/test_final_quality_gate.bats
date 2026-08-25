#!/usr/bin/env bats
@test "fixture" {
  run true
  [ "$status" -eq 0 ]
}
