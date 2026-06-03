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
