#!/usr/bin/env bats
# tests/unit/test_autonomy_io.bats — shared autonomy IO helpers
# (scripts/autospec_autonomy_io.py), extracted from autospec-autonomy-v2-lib.py.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPTS="$REPO_ROOT/scripts"
    WORK="$(mktemp -d)"
}

teardown() {
    rm -rf "${WORK:?}"
}

run_py() {
    run python3 -c "
import sys
sys.path.insert(0, '$SCRIPTS')
import autospec_autonomy_io as io
$1
"
}

@test "autonomy io: write_json round-trips sorted, indented JSON" {
    run_py "
from pathlib import Path
p = Path('$WORK/nested/out.json')
io.write_json(p, {'b': 1, 'a': 2})
print(p.read_text().strip())
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '\"a\": 2'
    # Keys are sorted and the parent directory is created on demand.
    [ "$(printf '%s\n' "$output" | sed -n '2p' | tr -d ' ,')" = '"a":2' ]
}

@test "autonomy io: write_text strips trailing blank lines and ends with one newline" {
    run_py "
from pathlib import Path
p = Path('$WORK/out.txt')
io.write_text(p, 'body\n\n\n')
print(repr(p.read_text()))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "'body\\\\n'"
}

@test "autonomy io: load_json returns the default for missing and non-object files" {
    run_py "
from pathlib import Path
missing = Path('$WORK/nope.json')
listy = Path('$WORK/list.json'); listy.write_text('[1,2]')
print(io.load_json(missing, {'d': 1}), io.load_json(listy, {'d': 2}))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "{'d': 1} {'d': 2}"
}

@test "autonomy io: yaml_scalar quotes strings but leaves booleans and numbers bare" {
    run_py "
print(io.yaml_scalar('true'), io.yaml_scalar('-3.5'), io.yaml_scalar('docs/**'))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'true -3.5 "docs/**"'
}

@test "autonomy io: yaml_scalar escapes embedded quotes and backslashes" {
    run_py "
print(io.yaml_scalar('a\"b\\\\c'))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'a\\"b\\\\c'
}

@test "autonomy io: slug collapses punctuation and falls back to 'item'" {
    run_py "
print(io.slug('Hello, World!'), io.slug('---'))
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'hello-world item'
}

@test "autonomy io: reports and state create their directories under the repo root" {
    run_py "
from pathlib import Path
r = io.reports(Path('$WORK')); s = io.state(Path('$WORK'))
print(r.is_dir(), s.is_dir(), r.name, s.name)
"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -Fqx 'True True reports state'
}
