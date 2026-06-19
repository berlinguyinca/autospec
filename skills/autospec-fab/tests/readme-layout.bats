#!/usr/bin/env bats
#
# readme-layout.bats — regression guard for the "Fab model directory & output
# layout" section in skills/autospec-fab/README.md.
#
# The per-model sidecar convention (release_gate_stages._SIBLING_INPUTS) and the
# Phase 5.5 output layout (fab-completeness.sh: gates/<model>/release-gate.json +
# renders/<model>/contact-sheet.html) live in code. This test fails closed if
# the documented layout section is removed or loses a key path token, so docs
# and code can't silently drift apart.

setup() {
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../../.." && pwd)"
  README="${REPO_ROOT}/skills/autospec-fab/README.md"
}

readme_has() {
  # Real file read, no mocks: grep the committed README for a literal token.
  grep -qF -- "$1" "$README"
}

@test "README has a Fab model directory & output layout section" {
  readme_has "## Fab model directory & output layout"
}

@test "README documents metadata.json + baseline.json model-dir contents" {
  readme_has "metadata.json"
  readme_has "baseline.json"
}

@test "README documents every stage sidecar from _SIBLING_INPUTS" {
  readme_has "circuit.json"
  readme_has "duct.json"
  readme_has "printer.json"
  readme_has "load.json"
  readme_has "flow.json"
}

@test "README documents the Phase 5.5 output path layout" {
  readme_has "gates/"
  readme_has "renders/"
  readme_has "release-gate.json"
  readme_has "contact-sheet.html"
}
