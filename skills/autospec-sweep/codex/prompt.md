
# autospec-sweep (harness-neutral)

Create and maintain the tracked project-level autospec configuration at
`.autospec/autospec.yml`, then use it to run recurring sweeps that keep specs,
documentation, tests, and code aligned with reality.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-sweep   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ]; then exit 0; fi
mkdir -p "$HOME/.autospec"
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
NOW=$(date -u +%s)
if [ -f "$LAST" ]; then
    PREV=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
        || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
    if [ "$((NOW - PREV))" -lt 86400 ]; then exit 0; fi
fi
if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "WARN: self-update skipped (concurrent update in progress)" >&2; exit 0
fi
trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT
date -u +'%Y-%m-%dT%H:%M:%SZ' > "$LAST.tmp" && mv "$LAST.tmp" "$LAST"
REMOTE=$(curl -fsSL --max-time 5 \
    "https://api.github.com/repos/berlinguyinca/autospec/commits/main" \
    2>/dev/null | jq -r '.sha // empty' 2>/dev/null | cut -c1-7)
if [ -z "$REMOTE" ]; then
    echo "WARN: self-update skipped (network); continuing on installed version" >&2; exit 0
fi
LOCAL=$(cat "$INSTALLED" 2>/dev/null || true)
if [ "$REMOTE" = "$LOCAL" ]; then exit 0; fi
curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh" \
    | bash -s -- --skill all --harness all --update >/dev/null 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then
    echo "WARN: self-update skipped (install rc=$RC); continuing on installed version" >&2; exit 0
fi
printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" && mv "$INSTALLED.tmp" "$INSTALLED"
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

## Self-update mode

If the feature-request argument is exactly `update` after trimming whitespace and
lowercasing, enter self-update mode and do not run a sweep:

1. Detect whether `autospec-sweep` is installed in any harness.
2. Run the canonical suite installer once with `--update`:
   `curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update`
3. Show a short before/after summary and stop.

## Invocation

```bash
/autospec-sweep init
/autospec-sweep configure
/autospec-sweep run
```

- `init` creates `.autospec/autospec.yml` on first use **and** verifies that
  every supported harness (Claude Code, Codex CLI, OpenCode) is wired up
  consistently — shared persistent memory at `docs/memory/`, a `Memory
  inventory` section in `AGENTS.md`, a `CLAUDE.md` pointer if missing, and the
  `~/.claude/projects/<slug>/memory` symlink resolving to `docs/memory/`.
- `configure` reads current repo findings and edits the tracked config.
- `run` executes the configured sweep and feeds resulting fixes back through
  `autospec-review`, specs, issues, and `autospec-run`.

## Harness detection

Resolve model tiers once at skill start:

1. Claude Code: `TIER_A` = Opus + maximum thinking; `TIER_B` = Sonnet + medium.
2. OpenCode: `TIER_A` = top task model + high reasoning; `TIER_B` = smaller task model + medium.
3. Codex CLI: `TIER_A` = current top GPT + high reasoning; `TIER_B` = current spark/cost-optimized Codex + medium.

If `TIER_B` is unavailable, silently retry the same delegated work with
`TIER_A` while preserving the task handoff.

## Required capabilities & harness adapter

| Capability | Claude Code | OpenCode | Codex CLI | Fallback if missing |
|---|---|---|---|---|
| Repo inspection | `Agent` Explore | read-only `task` | `rg`, `git`, `gh` | Inspect locally with bounded reads |
| Foreground delegation | `Agent` | `task` | native subagent/CLI | Do the synthesis in-thread |
| Ask questions | `AskUserQuestion` | inline prompt | inline prompt | Ask one concise question |
| Subagent model tier | Tier A/B per harness detection | Tier A/B per harness detection | Tier A/B per harness detection | Fall back UP, never down |
| Subagent dispatch policy   | per AGENTS.md decision matrix        | per AGENTS.md decision matrix            | per AGENTS.md decision matrix            | inline with main-session token cost                |

## First-run config

Before any sweep work, check for `.autospec/autospec.yml`.

If it is missing:

1. Run:
   `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-sweep-wizard.sh" init`
2. Ask only the wizard's small starter set unless repo findings require a
   project-specific question:
   - sweep profile, default `full`
   - environment safety, default `strict_isolation`
   - team personality, default `auto`
   - competitor research, default `false`
3. Commit `.autospec/autospec.yml` with the next autospec spec/config commit.
   It is project source, not local runtime state.

If the config exists, read it first and treat it as the source of truth. Existing
`.autospec/test.yml` or `.autospec/clone.yml` files are compatibility inputs only;
do not make them the new source of truth.

## Cross-harness setup verification (init mode)

`init` always runs the harness-setup doctor after the wizard, regardless of
whether `.autospec/autospec.yml` already existed. The doctor is idempotent —
re-running it on a fully-configured repo is a fast no-op:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/verify-harness-setup.sh"
```

It ensures all three supported harnesses see the same project guidance and the
same persistent memory:

| Harness     | Entry file        | Memory source                                 |
|-------------|-------------------|-----------------------------------------------|
| Claude Code | `CLAUDE.md`       | `~/.claude/projects/<slug>/memory` → symlink → `docs/memory/` |
| Codex CLI   | `AGENTS.md`       | `docs/memory/` (linked from AGENTS.md `## Memory inventory`) |
| OpenCode    | `AGENTS.md`       | `docs/memory/` (linked from AGENTS.md `## Memory inventory`) |

The doctor delegates the memory migration + AGENTS.md inventory block to
`auto-init-memory.sh`, then verifies five invariants:

1. `docs/memory/` exists and contains a `MEMORY.md` index.
2. `~/.claude/projects/<slug>/memory` is a symlink resolving to `docs/memory/`.
3. `AGENTS.md` exists at the repo root.
4. `AGENTS.md` contains a `## Memory inventory` section.
5. `CLAUDE.md` exists at the repo root — created as a one-line pointer to
   `AGENTS.md` if missing, so Claude Code converges on the same source of truth.

Run `verify-harness-setup.sh --check` (read-only) as a doctor at any time; run
without flags to apply fixes. Exit code is non-zero only when an unfixable
invariant fails.

## Configure mode

Use this mode when the user wants to modify sweep behavior or when the repo has
changed enough that the config is stale.

1. Inspect tracked specs, `AGENTS.md`, package manifests, test commands, E2E
   config, deployment docs, and existing `.autospec/*` compatibility files.
2. Compare findings with `.autospec/autospec.yml`.
3. Ask the minimum project-specific questions needed to resolve missing values.
4. Edit only the relevant config sections.
5. If a config change contradicts a tracked spec, update the spec in the same
   autospec design/spec PR before any implementation issue is filed.

## Parallel area dispatch

Sweep runs fan out into 4 parallel area subagents per the AGENTS.md subagent
dispatch decision matrix. Areas are defined under
`skills/autospec-sweep/areas/` and dispatched via
`scripts/sweep-area-dispatch.sh`:

1. `spec-vs-code-drift` — reuses `scripts/explore-research/spec-vs-code.sh`.
2. `docs-drift` — reuses `scripts/dogfood-adapter-doc-drift.sh`.
3. `code-health` — reuses `scripts/explore-research/codebase-signals.sh`.
4. `dependency-health` — new `scripts/explore-research/dependency-health.sh`
   researcher (also available to autospec-explore — extends the 6-researcher
   set to 7).

Each area subagent emits research-cycle JSON. The dispatcher aggregates into
`.autospec/sweep/area-findings.json` (`schema: autospec-sweep.area-findings.v1`)
which feeds the existing sweep report and `autospec-review`.

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/sweep-area-dispatch.sh"
```

## Sweep execution

> **Model tier:** TIER_A for sweep synthesis and gap classification. Any fix
> implementation created by downstream `autospec-run` uses that skill's Phase 4
> Tier B/Tier A routing.

Start with the executable runner:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-sweep-run.sh" run
```

The runner validates `.autospec/autospec.yml`, writes
`.autospec/sweep/latest.json`, accepts `--gaps <json>` for already-emitted gap
contracts, and executes the bundled `autospec-sweep-review.sh` by default. Set
`AUTOSPEC_SWEEP_REVIEW_CMD="<command>"` to replace the deterministic wrapper with
a harness-specific or richer review command.

Run the sweep from current reality back to source artifacts:

1. Read `.autospec/autospec.yml`.
2. Run the enabled review surfaces: spec coverage, docs drift, test gaps, code
   health, security footguns, clone/test readiness, and implementation status.
3. Emit concrete gaps with file paths, commands, and expected behavior.
4. Update or create specs before creating code-fix issues when reality and specs
   disagree.
5. File bounded `auto-implement` issues for accepted gaps and hand them to
   `autospec-run`.

## Continuous improvement

The default sweep is not a one-time bug hunt. It continuously improves:

- Documentation: keep user docs, API docs, runbooks, and generated docs in sync
  with implemented behavior.
- Documentation targets: treat `documentation.audiences[]` and
  `documentation.scopes[]` in `.autospec/autospec.yml` as the source of truth
  for deep docs. Emit separate gaps for missing target files or missing required
  `autospec-doc-scope` markers so user, developer, operator, security, runbook,
  API, and troubleshooting docs can be built as bounded reviewable work.
- Tests: add missing regression, unit, integration, E2E, invariant, and negative
  path coverage where the sweep finds weak confidence.
- Code: reduce duplication, dead code, complexity, brittle boundaries, and
  security-sensitive shortcuts in small reviewable issues.

Every sweep run must execute the configured full test commands. Do not treat
review-only sweeps as sufficient. If E2E or integration tests require the
software to be deployed or started first, run `project.findings.commands.deploy`
before those tests. A failing deploy, unit, integration, or E2E command fails the
sweep and must be fixed before filing success claims.

Prefer small, reversible changes. File specs and issues for improvement work; do
not silently refactor broad surfaces inside the sweep itself.

## Spec sync

Specs must stay synchronized with reality:

- If code behavior is correct and the spec is stale, update the spec first.
- If the spec is correct and code behavior is stale, file implementation issues.
- If both are ambiguous, ask one project-specific question and record the answer
  in the config or spec before proceeding.

## Safety

Never clone raw secrets or API keys. The config may store secret references,
environment variable names, and allowlisted settings, but not secret values.
Production-like sweeps default to strict isolation. Scoped production access
requires explicit config, backup/restore, and the existing autospec-test safety
rails.

## Verification

After changing config or sweep behavior, verify:

```bash
bash scripts/validate.sh
bats tests/unit/test_autospec_sweep_config.bats
bats tests/unit/test_autospec_sweep_run.bats
ajv compile -s schemas/autospec-config.schema.json --spec=draft2020
```

Report changed config sections, generated questions, and any remaining unknowns.
