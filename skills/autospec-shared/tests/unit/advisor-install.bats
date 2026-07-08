#!/usr/bin/env bats
# Ship-completeness guard for the advisor scripts.
#
# install.sh ships skills/autospec-shared/scripts/ wholesale via
#   cp -R "$shared_scripts_src/." "$autospec_scripts_dir/"
# so the advisor scripts are shipped iff (a) that wholesale copy is still how the
# installer ships shared scripts, and (b) the scripts exist and are executable in
# the source tree. This test asserts both without running the global installer.

setup() {
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../../../.." && pwd)"
  SRC="$REPO_ROOT/skills/autospec-shared/scripts"
}

@test "advisor-escalate.sh exists and is executable in source" {
  [ -x "$SRC/advisor-escalate.sh" ]
}

@test "advisor-report.sh exists and is executable in source" {
  [ -x "$SRC/advisor-report.sh" ]
}

@test "install.sh ships the shared scripts dir wholesale" {
  grep -Eq 'cp -R "\$shared_scripts_src/\." "\$autospec_scripts_dir/"' "$REPO_ROOT/install.sh"
}

@test "wholesale copy places advisor scripts, executable, in a target dir" {
  DEST="$(mktemp -d)"
  cp -R "$SRC/." "$DEST/"
  [ -x "$DEST/advisor-escalate.sh" ]
  [ -x "$DEST/advisor-report.sh" ]
  rm -rf "$DEST"
}
