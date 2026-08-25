#!/usr/bin/env bats
# doc-orchestrator.bats — TDD for skills/autospec-doc/scripts/doc-orchestrator.mjs (issue #916)
#
# The orchestrator is a router stub: it parses the §D1 subcommand contract,
# dispatches to distinct named handlers, and exits 2 when a non-`init`
# subcommand runs without a `documentation:` config.

ORCH="${BATS_TEST_DIRNAME}/../scripts/doc-orchestrator.mjs"

setup() {
  TESTDIR="$(mktemp -d)"
  cd "$TESTDIR"
}

teardown() {
  rm -rf "$TESTDIR"
}

# Seed a minimal .autospec/autospec.yml carrying a documentation: block so the
# config gate passes for non-init subcommands.
seed_config() {
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<'EOF'
documentation:
  audiences:
    - name: user
      path: docs/user
EOF
}

@test "init prints a scaffold plan and exits 0 (no config required)" {
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  [[ "$output" == *"init: scaffold plan"* ]]
}

@test "init works even with no documentation config present" {
  # No seed_config — init is the bootstrap that creates the config.
  run node "$ORCH" init
  [ "$status" -eq 0 ]
}

@test "bare invocation routes to the incremental handler" {
  seed_config
  run node "$ORCH"
  [ "$status" -eq 0 ]
  [[ "$output" == *"incremental:"* ]]
}

@test "--full routes to the full handler" {
  seed_config
  run node "$ORCH" --full
  [ "$status" -eq 0 ]
  [[ "$output" == *"full:"* ]]
}

@test "--audit routes to the audit handler" {
  seed_config
  run node "$ORCH" --audit
  [ "$status" -eq 0 ]
  [[ "$output" == *"audit:"* ]]
}

@test "--audience <name> routes to the audience handler with the name" {
  seed_config
  run node "$ORCH" --audience developer
  [ "$status" -eq 0 ]
  [[ "$output" == *"developer"* ]]
}

@test "bare, --full, --audit, --audience route to DISTINCT handlers" {
  seed_config
  bare="$(node "$ORCH" 2>&1)"
  full="$(node "$ORCH" --full 2>&1)"
  audit="$(node "$ORCH" --audit 2>&1)"
  aud="$(node "$ORCH" --audience admin 2>&1)"
  # All four outputs must differ from each other.
  [ "$bare" != "$full" ]
  [ "$bare" != "$audit" ]
  [ "$bare" != "$aud" ]
  [ "$full" != "$audit" ]
  [ "$full" != "$aud" ]
  [ "$audit" != "$aud" ]
}

@test "non-init subcommand with no documentation config exits 2" {
  # No seed_config.
  run node "$ORCH" --full
  [ "$status" -eq 2 ]
  [[ "$output" == *"init"* ]]
}

@test "bare incremental with no documentation config exits 2" {
  run node "$ORCH"
  [ "$status" -eq 2 ]
}

@test "--audience without a name exits 2" {
  seed_config
  run node "$ORCH" --audience
  [ "$status" -eq 2 ]
}

@test "--audience followed by a flag is a usage error, not an audience name" {
  seed_config
  run node "$ORCH" --audience --full
  [ "$status" -eq 2 ]
}

@test "unknown argument prints usage and exits 2" {
  seed_config
  run node "$ORCH" --bogus
  [ "$status" -eq 2 ]
  [[ "$output" == *"Usage:"* ]]
}

# ── init scaffolding (issue #917) ─────────────────────────────────────────────

@test "init writes the documentation: block with style and examples keys" {
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  [ -f .autospec/autospec.yml ]
  grep -q '^documentation:' .autospec/autospec.yml
  grep -q 'palette: light-blue' .autospec/autospec.yml
  grep -q 'verify: true' .autospec/autospec.yml
  grep -q 'sandbox: worktree' .autospec/autospec.yml
}

@test "init seeds the four default audiences in the written block" {
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  grep -q 'name: user' .autospec/autospec.yml
  grep -q 'name: developer' .autospec/autospec.yml
  grep -q 'name: admin' .autospec/autospec.yml
  grep -q 'name: general' .autospec/autospec.yml
}

@test "init creates per-audience starter scopes per the folder contract" {
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  [ -f docs/user/index.md ]
  [ -f docs/user/getting-started.md ]
  [ -f docs/developer/architecture/.gitkeep ]
  [ -f docs/developer/api/.gitkeep ]
  [ -f docs/admin/getting-started.md ]
  [ -f docs/general/index.md ]
  grep -q 'autospec-doc-scope: audience=user' docs/user/index.md
}

@test "init --dry-run writes nothing" {
  run node "$ORCH" init --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"no files written"* ]]
  [ ! -f .autospec/autospec.yml ]
  [ ! -d docs ]
}

@test "init is idempotent: existing documentation: block left untouched" {
  seed_config
  before="$(cat .autospec/autospec.yml)"
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  [[ "$output" == *"already present"* ]]
  after="$(cat .autospec/autospec.yml)"
  [ "$before" == "$after" ]
}

@test "init does not overwrite an existing human-owned doc file" {
  mkdir -p docs/user
  echo "HUMAN OWNED" > docs/user/index.md
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  grep -q 'HUMAN OWNED' docs/user/index.md
}

@test "after init, a non-init subcommand passes the config gate" {
  node "$ORCH" init >/dev/null
  run node "$ORCH" --full
  [ "$status" -eq 0 ]
  [[ "$output" == *"full:"* ]]
}

@test "init output round-trips: loadConfig parses the written block to four defaults" {
  node "$ORCH" init >/dev/null
  export CFG="$PWD/.autospec/autospec.yml"
  export CONFMOD="${BATS_TEST_DIRNAME}/../scripts/doc-config.mjs"
  run node -e '
    import(process.env.CONFMOD).then(m => {
      const c = m.loadConfig(process.env.CFG);
      const names = c.audiences.map(a => a.name).join(",");
      if (names !== "user,developer,admin,general") { console.error("names=" + names); process.exit(1); }
      if (c.style.palette !== "light-blue") process.exit(1);
      if (c.examples.verify !== true || c.examples.sandbox !== "worktree") process.exit(1);
      console.log("roundtrip-ok");
    }).catch(e => { console.error(e); process.exit(1); });
  '
  [ "$status" -eq 0 ]
  [[ "$output" == *"roundtrip-ok"* ]]
}

# ── init feature-inventory scaffolding (doc-scaffold.mjs) ──────────────────────

@test "init seeds .autospec/doc-features.json with feature slugs from topic docs" {
  mkdir -p docs/apps docs/algorithms
  echo '# Export' > docs/apps/export-pipeline.md
  echo '# Ingest' > docs/apps/ingest.md
  echo '# Peaks' > docs/algorithms/peak-detection.md
  echo '# index' > docs/apps/index.md
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  [ -f .autospec/doc-features.json ]
  [[ "$output" == *"scaffolded"* ]]
  grep -q '"slug": "export-pipeline"' .autospec/doc-features.json
  grep -q '"slug": "ingest"' .autospec/doc-features.json
  grep -q '"slug": "peak-detection"' .autospec/doc-features.json
  # index.md must NOT become a feature.
  ! grep -q '"slug": "index"' .autospec/doc-features.json
}

@test "init does NOT overwrite an edited doc-features.json on re-run" {
  mkdir -p .autospec docs/apps
  echo '# Export' > docs/apps/export.md
  echo '{"features":[{"slug":"hand-authored"}]}' > .autospec/doc-features.json
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  grep -q 'hand-authored' .autospec/doc-features.json
  ! grep -q 'export' .autospec/doc-features.json
}

@test "init --dry-run writes no feature inventory" {
  mkdir -p docs/apps
  echo '# Export' > docs/apps/export.md
  run node "$ORCH" init --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"scaffold"* ]]
  [ ! -f .autospec/doc-features.json ]
}

# ── §D6 incremental orchestrator tests (issue #943) ───────────────────────────

# §D6: bare invocation with zero changed scopes (AUTOSPEC_CHECK_DRIFT_SH stub
# that exits 0 = clean) must be a fast no-op — log line only, no regeneration.
@test "§D6: bare incremental with zero changed scopes is a fast no-op" {
  seed_config
  # Stub check-doc-drift.sh to exit 0 (clean — no drift).
  STUB_DRIFT="$TESTDIR/stub-drift-clean.sh"
  cat > "$STUB_DRIFT" <<'SH'
#!/usr/bin/env bash
echo '{"status":"clean","changed_scopes":[]}'
exit 0
SH
  chmod +x "$STUB_DRIFT"
  export AUTOSPEC_CHECK_DRIFT_SH="$STUB_DRIFT"
  run node "$ORCH"
  [ "$status" -eq 0 ]
  [[ "$output" == *"no changed scopes"* ]]
}

# §D6: bare invocation with one changed scope (AUTOSPEC_CHECK_DRIFT_SH exits 1
# with a changed_scopes list) must log the scope and regenerate only that scope.
@test "§D6: bare incremental with changed scopes logs and regenerates only those scopes" {
  seed_config
  STUB_DRIFT="$TESTDIR/stub-drift-changed.sh"
  cat > "$STUB_DRIFT" <<'SH'
#!/usr/bin/env bash
echo '{"status":"drift","changed_scopes":["docs/user/features/export-pipeline.md"]}'
exit 1
SH
  chmod +x "$STUB_DRIFT"
  export AUTOSPEC_CHECK_DRIFT_SH="$STUB_DRIFT"
  run node "$ORCH"
  [ "$status" -eq 0 ]
  [[ "$output" == *"1 changed scope"* ]]
  [[ "$output" == *"export-pipeline"* ]]
}

# §D6: --full must NOT be affected by the incremental scope computation — it
# always regenerates everything regardless of check-doc-drift output.
@test "§D6: --full ignores changed-scope computation and does full fan-out" {
  seed_config
  STUB_DRIFT="$TESTDIR/stub-drift-clean.sh"
  cat > "$STUB_DRIFT" <<'SH'
#!/usr/bin/env bash
echo '{"status":"clean","changed_scopes":[]}'
exit 0
SH
  chmod +x "$STUB_DRIFT"
  export AUTOSPEC_CHECK_DRIFT_SH="$STUB_DRIFT"
  run node "$ORCH" --full
  [ "$status" -eq 0 ]
  [[ "$output" == *"full:"* ]]
  # full output must NOT contain "no changed scopes" (that's incremental-only messaging)
  [[ "$output" != *"no changed scopes"* ]]
}

# ── Generation wiring (issue #918/#923) ────────────────────────────────────────

# Seed a documentation config with the four default audiences plus a 1-feature
# inventory file. Generation must write into the TEMP project, never the repo.
seed_gen_config() {
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<'EOF'
documentation:
  audiences:
    - {name: user, path: docs/user, focus: "tasks"}
    - {name: developer, path: docs/developer, focus: "architecture"}
EOF
  cat > .autospec/doc-features.json <<'EOF'
{
  "features": [
    {
      "slug": "export-pipeline",
      "title": "Export Pipeline",
      "summary": {
        "user": "Export your data to CSV.",
        "developer": "Streaming export pipeline with backpressure."
      },
      "spec_sections": ["Configure the destination.", "Trigger the export."]
    }
  ]
}
EOF
}

@test "--full writes audience feature pages into the project (not the repo)" {
  # Record the autospec repo state so we can prove no pollution.
  REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../../.." && pwd)"
  before_repo="$(cd "$REPO_ROOT" && git status --porcelain)"

  seed_gen_config
  run node "$ORCH" --full
  [ "$status" -eq 0 ]
  [[ "$output" == *"full:"* ]]

  # Pages were written into the TEMP project under docs/<audience>/features/.
  [ -f docs/user/features/export-pipeline.md ]
  [ -f docs/developer/features/export-pipeline.md ]
  [ -s docs/user/features/export-pipeline.md ]
  [ -s docs/developer/features/export-pipeline.md ]
  grep -q 'autospec-doc-scope' docs/user/features/export-pipeline.md

  # The autospec repo must be byte-for-byte unchanged (projectRoot() guard).
  after_repo="$(cd "$REPO_ROOT" && git status --porcelain)"
  [ "$before_repo" == "$after_repo" ]
}

@test "--audit writes nothing and reports audit:" {
  seed_gen_config
  # Snapshot the temp tree.
  snapshot_before="$(find . -type f | sort; find . -type f -exec shasum {} \; | sort)"
  run node "$ORCH" --audit
  [ "$status" -eq 0 ]
  [[ "$output" == *"audit:"* ]]
  snapshot_after="$(find . -type f | sort; find . -type f -exec shasum {} \; | sort)"
  [ "$snapshot_before" == "$snapshot_after" ]
}

@test "--audience developer generates only developer pages" {
  seed_gen_config
  run node "$ORCH" --audience developer
  [ "$status" -eq 0 ]
  [[ "$output" == *"audience:"* ]]
  [ -d docs/developer ]
  [ -f docs/developer/features/export-pipeline.md ]
  # The user audience must NOT have been generated.
  [ ! -d docs/user ]
}

@test "--audience with an unknown name exits nonzero" {
  seed_gen_config
  run node "$ORCH" --audience nonsense
  [ "$status" -ne 0 ]
  [[ "$output" == *"not a known audience"* ]]
}

@test "generation keeps the command successful when llms-full output cannot be written" {
  seed_gen_config
  # A directory at the output path deterministically makes writeLlmsFull fail
  # with EISDIR; the orchestrator must report the warning without losing the
  # audience generation result.
  mkdir llms-full.txt
  run node "$ORCH" --audience developer
  [ "$status" -eq 0 ]
  [[ "$output" == *"audience: regenerated"* ]]
  [[ "$output" == *"llms-full regen error"* ]]
  [ -s docs/developer/features/export-pipeline.md ]
}

# ── Answerability / domain-term coverage audit (doc-coverage.mjs) ───────────────

# Seed a config + a source file containing a SCREAMING_SNAKE enum constant that
# NO generated doc mentions. The audit must mine it and report it missing.
seed_coverage_config() {
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<EOF
documentation:
  audiences:
    - {name: user, path: docs/user, focus: "tasks"}
$1
EOF
  # A source file with a high-signal enum across two files so it passes
  # min_freq>=3 / min_files>=2 thresholds.
  mkdir -p src
  cat > src/Annotation.scala <<'EOF'
object Annotation {
  val a = AnnotationTargetType.INVALID_TARGET
  val b = INVALID_TARGET
}
EOF
  cat > src/Target.scala <<'EOF'
object Target {
  val x = INVALID_TARGET
}
EOF
  # A generated user doc that does NOT mention INVALID_TARGET.
  mkdir -p docs/user/features
  cat > docs/user/features/export-pipeline.md <<'EOF'
<!-- autospec-doc-scope: audience=user -->
# Export Pipeline
Export your data to CSV.
EOF
}

@test "--audit reports a domain coverage section and lists the missing enum" {
  seed_coverage_config ""
  run node "$ORCH" --audit
  [ "$status" -eq 0 ]
  [[ "$output" == *"domain coverage:"* ]]
  [[ "$output" == *"INVALID_TARGET"* ]]
  [[ "$output" == *"missing enum:"* ]]
}

@test "--full prints a one-line domain coverage summary" {
  seed_coverage_config ""
  run node "$ORCH" --full
  [ "$status" -eq 0 ]
  [[ "$output" == *"full:"* ]]
  [[ "$output" == *"domain coverage:"* ]]
  [[ "$output" == *"high-signal terms missing"* ]]
}

@test "coverage enabled:false suppresses the domain coverage output" {
  seed_coverage_config "  coverage:
    enabled: false"
  run node "$ORCH" --audit
  [ "$status" -eq 0 ]
  [[ "$output" == *"audit:"* ]]
  [[ "$output" != *"domain coverage:"* ]]
}

@test "coverage exclude_globs from config flows through to the extractor" {
  # Proves the orchestrator forwards the newer override knobs (not just
  # min_freq/stoplist): excluding src/** must drop the INVALID_TARGET enum.
  seed_coverage_config "  coverage:
    exclude_globs: [\"src/**\"]"
  run node "$ORCH" --audit
  [ "$status" -eq 0 ]
  [[ "$output" == *"domain coverage:"* ]]
  [[ "$output" != *"INVALID_TARGET"* ]]
}

# ── Single-file audience mode in the orchestrator (issue #2968) ───────────────
#
# A .md-suffixed audience path renders ONE composed document at that exact
# path (features folded in). The page collector must count the file itself,
# and the completeness audit must not expect per-feature folder pages for it.

seed_single_file_config() {
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<'EOF'
documentation:
  audiences:
    - {name: user, path: docs/user, focus: "tasks"}
    - {name: security, path: docs/SECURITY.md, focus: "security"}
EOF
  cat > .autospec/doc-features.json <<'EOF'
{
  "features": [
    {
      "slug": "audit-log",
      "title": "Audit Log",
      "summary": {
        "user": "See what happened to your data.",
        "security": "Tamper-evident audit trail."
      },
      "spec_sections": ["Events are recorded immutably."]
    }
  ]
}
EOF
}

@test "--full renders a .md audience as one document and audits 0 missing" {
  seed_single_file_config
  run node "$ORCH" --full
  [ "$status" -eq 0 ]
  # The .md path is a FILE, not a folder subtree.
  [ -f docs/SECURITY.md ]
  [ ! -d docs/SECURITY.md ]
  [ ! -e docs/SECURITY.md/features ]
  # The folder audience still gets its per-feature page.
  [ -f docs/user/features/audit-log.md ]
  # The audit summary printed by --full reports zero missing pages.
  [[ "$output" == *"0 missing page(s)"* ]]
}

@test "--audit does not expect per-feature folder pages for a .md audience" {
  seed_single_file_config
  node "$ORCH" --full >/dev/null
  run node "$ORCH" --audit
  [ "$status" -eq 0 ]
  # No false "missing" finding for the folded feature page.
  [[ "$output" != *"docs/SECURITY.md/features/audit-log.md"* ]]
  [[ "$output" == *"0 missing page(s)"* ]]
}

@test "page collector ignores a non-.md file audience (renderer .md contract)" {
  # A file at a non-.md audience path is a misconfiguration: the renderer
  # keeps the folder contract for it, so the collector must not count the
  # file itself as an audience page either.
  mkdir -p .autospec docs
  cat > .autospec/autospec.yml <<'EOF'
documentation:
  audiences:
    - {name: notes, path: docs/NOTES.txt, focus: "notes"}
EOF
  echo "hello world, a short note" > docs/NOTES.txt
  run node "$ORCH" --audit
  [ "$status" -eq 0 ]
  [[ "$output" == *"0 page(s) on disk"* ]]
  [[ "$output" != *"docs/NOTES.txt"* ]]
}
