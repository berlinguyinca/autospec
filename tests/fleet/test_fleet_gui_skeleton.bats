#!/usr/bin/env bats
# tests/fleet/test_fleet_gui_skeleton.bats — the config the fleet GUI creates must lint.
#
# #2955 renamed the sample catalog's local profile and updated every consumer except this
# one: `DEFAULT_SKELETON["default_profile"]` in fleet-gui-server.py still named a key that
# no longer existed, so every config the GUI created failed `fleet-config-lint.sh` with
# `unknown profile`. It stayed broken because the pre-commit gate rejected the file on
# pre-existing COMPLEXITY findings that no edit could clear (#2961).
#
# The existing suite passes because it never lints the config it builds — it asserts
# `'default_profile' in cfg`, which holds for any string at all. This file closes that gap
# by reading the skeleton the server actually declares and putting it through the real
# linter, so the next rename fails here rather than in an operator's terminal.
#
# A separate file rather than a case appended to test_fleet_gui.bats: that suite is 1,184
# lines, over the CI file-size ratchet's limit, so it may not grow.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SERVER="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-gui-server.py"
    LINT="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-config-lint.sh"
    WORK="$(mktemp -d)"
}

teardown() {
    rm -rf "${WORK:?}"
}

# Extracts DEFAULT_SKELETON from the server source and writes it as YAML. Parsed out of
# the module rather than restated here: a copy in the test would keep passing after the
# server's value drifted, which is the failure this file exists to catch.
write_skeleton_config() {
    python3 - "$SERVER" "$WORK/autospec-fleet.yml" <<'PY'
import ast
import sys

source, dest = sys.argv[1], sys.argv[2]
tree = ast.parse(open(source, encoding="utf-8").read())
assigned = None
for node in ast.walk(tree):
    if isinstance(node, ast.Assign) and any(
        isinstance(t, ast.Name) and t.id == "DEFAULT_SKELETON" for t in node.targets
    ):
        assigned = node.value
        break
if assigned is None:
    raise SystemExit("DEFAULT_SKELETON not found in " + source)

# This walked the syntax tree node by node until #3057, because SECURITY read the stdlib
# literal parser's name as the dangerous builtin it is the safe alternative to, and no
# annotation could reach the rule. Both are fixed, so the direct call is back — and its
# presence here is the end-to-end regression test for that fix.
skeleton = ast.literal_eval(assigned)

lines = []
for key, value in skeleton.items():
    if isinstance(value, list):
        if value:
            raise SystemExit("skeleton list values are expected to be empty: %r" % (value,))
        # The schema requires at least one repo, so the empty skeleton alone can never
        # validate — it is a starting point the operator fills in. One placeholder repo
        # is added here so the assertion is about the profile name, which is what #2961
        # is about, rather than about a `repos` list the GUI is not expected to populate.
        lines.append("%s:" % key)
        lines.append('  - url: "https://github.com/berlinguyinca/autospec"')
    elif isinstance(value, bool):
        lines.append("%s: %s" % (key, "true" if value else "false"))
    elif isinstance(value, int):
        lines.append("%s: %d" % (key, value))
    else:
        lines.append('%s: "%s"' % (key, value))
open(dest, "w", encoding="utf-8").write("\n".join(lines) + "\n")
print(skeleton["default_profile"])
PY
}

@test "the skeleton the GUI writes passes fleet-config-lint with no profile override" {
    run write_skeleton_config
    [ "$status" -eq 0 ]

    # No --profiles: the resolution order under test is the default one, which prefers
    # examples/model-profiles.yml over ~/.autospec/model-profiles.yml. Passing an
    # override here would test a catalog the GUI's users never get.
    run bash -c "cd '$REPO_ROOT' && bash '$LINT' --config '$WORK/autospec-fleet.yml'"
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'unknown profile'
}

@test "the skeleton's default_profile is a key the sample catalog actually defines" {
    local profile
    profile="$(write_skeleton_config)"
    [ -n "$profile" ]
    grep -qE "^${profile}:" "$REPO_ROOT/examples/model-profiles.yml"
}
