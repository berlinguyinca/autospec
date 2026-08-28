---
description: Use when Autospec runtime provisioning detects Docker Compose files that require migration for isolated worktree environments.
mode: primary
---

# Autospec Compose normalization

Orchestrate one fingerprinted prerequisite migration. Delegate every YAML decision and
edit to the Rust `normalize-compose` command; never implement Compose policy in the
prompt, shell, or model.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-compose-normalize -->

## Required capabilities and harness adapter

| Capability | Contract |
|---|---|
| Shell | Run direct argument vectors and preserve command exit status. |
| GitHub | Use `gh` for issue, pull-request, review, CI, and merge operations. |
| Worktrees | Use `worktree-guard.sh`; never mutate the primary checkout. |
| Claims | Use `claim-guard.sh` in strict, all-or-nothing mode. |
| Subagent dispatch policy | Follow the AGENTS.md subagent-vs-inline decision matrix. |

## Invocation mode

- **Autospec-managed invocation:** run the complete workflow through merge, refresh
  the caller from `origin/main`, then retry runtime provisioning.
- **Direct unmanaged invocation:** perform the read-only check and lookup only. If a
  migration is required, print the matching issue and pull-request URLs (or explicitly
  report that neither exists) and exit before any `autospec runtime env up` command.
  Do not acquire claims, create a worktree, apply edits, or claim isolation.

The caller determines the mode from its own workflow context. Do not invent an
environment variable to infer it.

## 1. Check and validate the plan

Run the check unconditionally from the current Git worktree, even when neither runtime
manifest exists yet:

```bash
PLAN_FILE=$(mktemp "${TMPDIR:-/tmp}/autospec-compose-plan.XXXXXX")
ERROR_FILE="${PLAN_FILE}.err"
if ! autospec runtime env normalize-compose --repo "$PWD" --check >"$PLAN_FILE" 2>"$ERROR_FILE"; then
  if grep -q 'NORMALIZE_COMPOSE_NOT_FOUND' "$ERROR_FILE"; then
    rm -f "$PLAN_FILE" "$ERROR_FILE"
    exit 0
  fi
  sed -n '1,120p' "$ERROR_FILE" >&2
  rm -f "$PLAN_FILE" "$ERROR_FILE"
  exit 1
fi
```

Read the versioned JSON deterministically. Require `schema_version` to equal `1`,
`fingerprint` to be exactly 64 lowercase hexadecimal characters, and
`remaining_diagnostics` to be empty. A non-empty diagnostic list is terminal and must
be printed verbatim; do not create a migration. Treat malformed JSON and unknown schema
versions as terminal. Treat `NORMALIZE_STALE_SOURCE` from any later command as terminal
and restart from a fresh check.

Use Python's standard JSON parser rather than line parsing or an external JSON tool:

```bash
FINGERPRINT=$(python3 - "$PLAN_FILE" <<'PY'
import json, re, sys
plan = json.load(open(sys.argv[1], encoding="utf-8"))
fingerprint = plan.get("fingerprint", "")
if plan.get("schema_version") != 1:
    raise SystemExit("unsupported normalization schema_version")
if plan.get("remaining_diagnostics"):
    raise SystemExit(json.dumps(plan["remaining_diagnostics"], indent=2))
if not re.fullmatch(r"[0-9a-f]{64}", fingerprint):
    raise SystemExit("fingerprint must be 64 lowercase hexadecimal characters")
print(fingerprint)
PY
)
```

If `edits` is empty, delete the temporary files and return success. Otherwise continue
with the exact marker `<!-- autospec-compose-fingerprint: SHA256 -->`, replacing
`SHA256` with `$FINGERPRINT` in every issue and pull-request body.

## 2. Reuse before creating

Search issue bodies and pull-request bodies in the current repository for the exact
fingerprint marker. Inspect complete JSON results so a substring or title match cannot
select another migration.

```bash
MARKER="<!-- autospec-compose-fingerprint: $FINGERPRINT -->"
WORKFLOW_GUARD="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-compose-normalize-guard.sh"
LOOKUP_RESULTS=$("$WORKFLOW_GUARD" lookup "$FINGERPRINT")
```

The guard searches the exact 64-character fingerprint in each body, then filters for
the full marker. Its bounded result set therefore cannot be displaced by hundreds of
unrelated generic Compose migrations.

- For a matching open issue or pull request, reuse it. A managed invocation waits for
  its normal completion and retries the check; a direct invocation prints every known
  URL and exits.
- For a closed issue, run `gh issue reopen <number>` and reuse it. For a closed,
  unmerged pull request, run `gh pr reopen <number>` and reuse it when GitHub permits.
  If reopening the pull request fails, print its URL as a terminal recovery blocker;
  never create a replacement issue or pull request for that fingerprint.
- For a matching merged pull request, fetch and merge current `origin/main` into the
  active feature worktree, then rerun the check before `env up`.
- Never create another migration when either exact match exists.
- If no match exists, a direct invocation reports that state and exits. Only an
  Autospec-managed invocation continues.

For every direct unmanaged invocation, run `"$WORKFLOW_GUARD" direct-refuse
"$FINGERPRINT"` and return on its expected exit status `3`. This command prints all
matching URLs (or the explicit no-match state) and never invokes runtime provisioning.

This is the exactly one migration issue, branch, worktree, and pull request invariant
for a fingerprint.

## 3. Claim the complete input surface

Consume only the Rust plan's repo-relative `input_paths` and `manifest_path`. Include
the selected manifest even when it does not exist yet, so its creation is leased. Do
not rediscover inputs with Git, filesystem globs, or YAML parsing.

```bash
CLAIM_FILE="${PLAN_FILE}.claims"
python3 - "$PLAN_FILE" "$CLAIM_FILE" <<'PY'
import json, pathlib, sys
plan = json.load(open(sys.argv[1], encoding="utf-8"))
inputs = plan.get("input_paths")
manifest = plan.get("manifest_path")
if not isinstance(inputs, list) or not all(isinstance(p, str) for p in inputs):
    raise SystemExit("normalization plan input_paths must be a string array")
if not isinstance(manifest, str) or not manifest:
    raise SystemExit("normalization plan manifest_path must be a non-empty string")
paths = sorted(set([manifest, *inputs]))
for value in paths:
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or "\n" in value or "\t" in value:
        raise SystemExit(f"normalization claim path is not repo-relative: {value!r}")
with open(sys.argv[2], "w", encoding="utf-8") as output:
    output.write("\n".join(paths) + "\n")
PY
SELECTED_MANIFEST=$(python3 - "$PLAN_FILE" <<'PY'
import json, sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["manifest_path"])
PY
)
CLAIM_INPUTS=$(sed '/^$/d' "$CLAIM_FILE")
OLD_IFS=$IFS
IFS='
'
set -- $CLAIM_INPUTS
IFS=$OLD_IFS
CLAIMS_HELD=0
COMPOSE_CLAIM_SESSION="${AUTOSPEC_SESSION_ID:-}"
if [ -z "$COMPOSE_CLAIM_SESSION" ]; then
  COMPOSE_CLAIM_SESSION=$("$WORKFLOW_GUARD" new-session-token)
fi
release_compose_claims() {
  if [ "${CLAIMS_HELD:-0}" = 1 ]; then
    "$WORKFLOW_GUARD" claim release "$COMPOSE_CLAIM_SESSION" "$@"
    CLAIMS_HELD=0
  fi
}
trap 'release_compose_claims "$@"' EXIT HUP INT TERM
"$WORKFLOW_GUARD" claim acquire "$COMPOSE_CLAIM_SESSION" "$@"
CLAIMS_HELD=1
"$WORKFLOW_GUARD" claim verify "$COMPOSE_CLAIM_SESSION" "$@"
```

The acquire is all-or-nothing. On conflict, release nothing, print the owning session,
and retry lookup later. The status verification turns claim-guard's deliberately
fail-open unwritable-store mode into a fail-closed migration prerequisite. Hold and
refresh the claims through merge. Call
`release_compose_claims "$@"` on success and rely on the trap on every terminal path.

## 4. Create one classified migration

After claims succeed, repeat the exact fingerprint lookup. This closes the race between
lookup and acquire. If a match appeared, release and reuse it.

```bash
LOOKUP_RESULTS_AFTER_CLAIM=$("$WORKFLOW_GUARD" lookup "$FINGERPRINT")
```

Create one issue with label `needs-classify`; do not add `auto-implement` directly. Its
body must include the fingerprint marker and these lintable sections:

- `## Goal`: one concrete sentence naming the selected manifest and Compose migration.
- `## Acceptance criteria`: checkbox lines with paths, commands, or numeric results.
- `## Implementation outline`: every claimed file that the migration may touch.
- `## Tests required`: real Compose validation and the repository full suite.
- `### Primary smoke test (inner loop)`: one command line only.

Run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-issue.sh` on the body
before `gh issue create`. Then run
`autospec-classify` for the new issue and verify `needs-classify` was replaced by
`auto-implement` with both `ctx:*` and `reasoning:*` labels before implementation.

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-issue.sh" "$ISSUE_BODY"
ISSUE_URL=$(gh issue create --title "feat: migrate Compose isolation $FINGERPRINT" \
  --body-file "$ISSUE_BODY" --label needs-classify)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/project-sync-issue.sh" "$ISSUE_URL" "$PWD"
ISSUE_NUMBER=${ISSUE_URL##*/}
```

Invoke the installed classifier through the current harness, substituting the concrete
number (never dispatch a symbolic command):

```text
/autospec-classify --issues <ISSUE_NUMBER>
```

Use branch `feat/compose-isolation-$FINGERPRINT` truncated only after enough digest
characters remain unique. Create it from current `origin/main` in one linked worktree:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh" create \
  --branch "feat/compose-isolation-${FINGERPRINT}" \
  --path "/tmp/wt-compose-isolation-${FINGERPRINT}"
```

Exactly one worktree owns implementation. Never apply in the caller's feature worktree.

## 5. Apply, prove, review, and merge

Inside the migration worktree, rerun `--check` and require the fingerprint to equal the
claimed value. Then delegate the only writes to Rust:

```bash
autospec runtime env normalize-compose --repo "$PWD" --apply --fingerprint "$FINGERPRINT"
autospec runtime env normalize-compose --repo "$PWD" --check
```

The second check must have no edits or remaining diagnostics. It also proves the Rust
normalizer can resolve `docker compose --profile "*" --all-resources ... config --format
json`. Run the repository full suite, commit the changed Compose/manifest files, and
push the single branch. Open one pull request containing the same fingerprint marker,
the issue-closing reference, exact verification evidence, and scoped file list.

```bash
PR_URL=$(gh pr create --title "feat: migrate Compose isolation $FINGERPRINT" \
  --body-file "$PR_BODY" --base main \
  --head "feat/compose-isolation-${FINGERPRINT}")
```

Run normal CI and review gates: `gh pr checks`, the repository implementation linter,
and a fresh reviewer that must return `LGTM`. Fix findings on new commits and repeat.
At every existing CI/heartbeat poll, refresh the same lease without starting another
background loop:

```bash
"$WORKFLOW_GUARD" claim refresh "$COMPOSE_CLAIM_SESSION" "$@"
```

Merge only after required CI and review pass, using the repository's authorized
`gh pr merge` mode. Do not release claims merely because the pull request opened.

After merge is confirmed, release claims, remove the migration worktree, fetch
`origin/main`, merge it into the caller's feature worktree, rerun this skill's check,
and only then continue to `autospec runtime env up`.

## Common failures

| Signal | Action |
|---|---|
| `NORMALIZE_COMPOSE_NOT_FOUND` | Return success; no migration is needed. |
| Remaining diagnostics | Print them and stop; no model-authored YAML workaround. |
| Stale fingerprint/source | Discard the plan and restart from `--check`. |
| Existing fingerprint marker | Reuse the issue/PR; never create a sibling migration. |
| Claim conflict | Wait and repeat lookup; never weaken strict mode. |
| Failed CI or non-LGTM review | Keep the claim, fix normally, and rerun the gates. |
