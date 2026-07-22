#!/usr/bin/env bats

DISCOVER="$BATS_TEST_DIRNAME/../../skills/autospec-harmonize/scripts/design-discover.sh"

@test "design discover avoids debug logging APIs in embedded node renderers" {
  ! grep -Eq 'console\.(log|debug|info|warn|error)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)' "$DISCOVER"
}
