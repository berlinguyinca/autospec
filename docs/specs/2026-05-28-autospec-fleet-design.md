# Autospec Fleet Design

Date: 2026-05-28

## Goal

Add `autospec-fleet`, a workspace-level supervisor that can start from an empty
directory, clone or sync multiple GitHub repositories from URLs, and launch the
existing `/autospec-run` monitor inside each repository so a group of machines
can work autospec issue queues continuously.

## Problem

`/autospec-run` already coordinates multiple workers against one checked-out
repository. It uses GitHub issue labels, dependency checks, path-conflict checks,
and `autospec-run-state` comments as the shared queue protocol. That solves the
per-repo locking problem, but operators still need a higher-level way to run a
cluster against many repositories.

The desired operator flow is:

```text
mkdir fleet-workspace
cd fleet-workspace
/autospec-fleet init https://github.com/org/repo-a https://github.com/org/repo-b
/autospec-fleet run
```

Long term, many nodes should be able to run this command 24/7. Each node should
pick repositories it is allowed to work on, respect its local capacity and model
profile, and rely on `/autospec-run` for issue-level claiming and merge flow.

## Team Personality

Selected team: **Reliability/backend fleet orchestration team**

Roles:

- platform engineer
- backend developer
- sysadmin/SRE
- security advisor
- test engineer
- documentation owner

This team fits because the feature is a distributed operations layer. The risks
are unsafe config distribution, duplicate local clones, competing schedulers,
credential leakage, unclear stop behavior, and poor status visibility. The team
emphasis for child issues is conservative shell orchestration, GitHub-native
state, explicit schemas, safe defaults, and strong operator docs.

### Review Counter-Team

Counter-team: **Maintainer and operator UX review team**

Roles:

- maintainer
- product-operations reviewer
- documentation owner
- regression-test reviewer

This team should challenge whether the new skill adds a needless control plane,
whether command names and config files are understandable, whether single-repo
`/autospec-run` behavior remains unchanged, and whether recovery is documented
well enough for unattended nodes. Review must stay inside fleet orchestration
and avoid reopening the already-solved per-repo claim protocol unless a fleet
requirement exposes a real gap.

## Architecture

Introduce a new top-level skill family:

```text
skills/autospec-fleet/
  SKILL.md
  README.md
  codex/prompt.md
  opencode/agent.md
  install.sh
  uninstall.sh
  scripts/
    fleet-init.sh
    fleet-run.sh
    fleet-status.sh
    fleet-stop.sh
    fleet-config-lint.sh
```

`autospec-fleet` is a supervisor, not a replacement for `/autospec-run`.

- Fleet owns workspace discovery, repo clone/sync, repo-level scheduling, and
  aggregate status.
- `/autospec-run` owns issue selection, claim/release, implementation, review,
  CI waiting, merge, and per-repo final reporting.
- GitHub remains the shared queue and lock layer for issues.
- A Git control repository is the recommended distribution mechanism for fleet
  desired state. Nodes pull it periodically or on operator command.
- Node-local capacity stays in `~/.autospec/fleet-node.yml` and is never
  committed.

This keeps the first version useful on one laptop while giving a clean path to a
cluster of unattended workers.

## API Shape

Primary commands:

```text
/autospec-fleet init <repo-url>...
/autospec-fleet sync [--config-repo <url>]
/autospec-fleet run [--profile <name>] [--parallel <N>] [--once]
/autospec-fleet status
/autospec-fleet stop --graceful
/autospec-fleet stop --immediate
```

`init` creates `autospec-fleet.yml` in the current directory and clones the
listed repositories under the configured workspace. `sync` updates the local
copy of a fleet control repo when configured. `run` loops through enabled repos,
launching `/autospec-run` in each eligible checkout. `status` summarizes repo
queues, active workers, open PRs, and recent failures. `stop` forwards the
existing autospec stop sentinel to every active repo runner.

Optional flags:

- `--repo <url>` on `run` appends an ad hoc repo without editing config.
- `--config <path>` selects a non-default fleet config file.
- `--config-repo <url>` clones or fetches the canonical desired-state repo.
- `--workspace <path>` overrides the clone directory.
- `--worker-id-prefix <id>` prefixes per-repo worker IDs.
- `--dry-run` prints clone and `/autospec-run` commands without running them.

## Data Model

Fleet desired state lives in a versioned YAML file:

```yaml
version: 1
workspace: .autospec-fleet/repos
default_profile: qwen3-32b-laptop
parallel_repos: 2
sync:
  config_repo: git@github.com:org/autospec-fleet-control.git
  ref: main
repos:
  - url: https://github.com/org/repo-a.git
    profile: qwen3-32b-laptop
    enabled: true
  - url: git@github.com:org/repo-b.git
    profile: claude-sonnet-cloud
    enabled: true
```

Node-local capacity lives outside the repo:

```yaml
node_id: mac-mini-01
workspace: ~/.autospec/fleet/repos
max_parallel_repos: 2
profiles:
  - qwen3-32b-laptop
```

Runtime status lives under `~/.autospec/fleet/runs/<run-id>.json`:

```json
{
  "schema": 1,
  "run_id": "20260528T120000Z-mac-mini-01",
  "node_id": "mac-mini-01",
  "started_at": "2026-05-28T12:00:00Z",
  "repos": [
    {
      "repo": "org/repo-a",
      "path": "/Users/operator/.autospec/fleet/repos/org__repo-a",
      "profile": "qwen3-32b-laptop",
      "state": "running",
      "last_command": "/autospec-run --profile qwen3-32b-laptop --worker-id fleet:mac-mini-01:org__repo-a"
    }
  ]
}
```

Repository URL normalization maps:

- `https://github.com/org/repo`
- `https://github.com/org/repo.git`
- `git@github.com:org/repo.git`

to `owner/repo` plus a filesystem slug `owner__repo`.

## Configuration Distribution

Fleet configuration is split into two layers.

1. **Desired state, shared and versioned.** The canonical
   `autospec-fleet.yml` lives in a Git control repository. Operators update it
   by PR, CI validates it, and nodes fetch it on `sync` or at each scheduling
   tick.
2. **Node-local capacity, private.** `~/.autospec/fleet-node.yml` declares node
   identity, workspace path, local model profiles, and concurrency limits. It is
   never committed and never contains secrets.

This avoids a new control plane in v1. Git provides review, audit history,
rollback, auth, and distribution. GitHub Issues plus the existing
`autospec-run-state` comments remain the runtime coordination layer.

## Scheduling

The scheduler is intentionally simple in v1:

1. Load desired state and node-local capacity.
2. Clone missing repos and `git fetch` existing repos.
3. For each enabled repo, run a cheap queue probe:
   `list-ready-issues.sh --repo <owner/repo> --batch-size 1`.
4. Skip repos with no ready work, no matching profile, missing auth, dirty local
   checkout, or disabled config.
5. Launch up to `min(parallel_repos, max_parallel_repos)` repo workers.
6. Each repo worker runs `/autospec-run --profile <profile> --worker-id <id>` in
   that checkout.
7. Re-scan after a worker exits or after a bounded polling interval.

The fleet scheduler never claims individual issues. It may call
`/autospec-run --coordination-status` and `--max-parallel-safe` for visibility,
but all issue ownership stays with `/autospec-run`.

## Error Handling

- Invalid fleet config: fail before cloning or launching workers.
- Missing `gh` auth: print the failing repo and stop that node run.
- Missing repo access: mark the repo `auth_failed` in fleet status and continue
  with other repos.
- Clone failure: retry once; then mark `clone_failed`.
- Dirty managed checkout: skip and report `dirty_checkout`; never reset.
- Config repo fetch failure: continue with last synced config and warn.
- Unknown profile: skip repos using it and print available profile names.
- Usage limit pause: preserve `/autospec-run` usage-limit recovery; fleet records
  the resume command per repo worker.
- Stop graceful: stop launching new repo workers and let active repo workers
  finish their current issue.
- Stop immediate: forward the existing immediate stop sentinel and write fleet
  status before exiting.

## Security

Fleet must never copy secrets from target repos into the control repo. Config
may contain repo URLs, profile names, and scheduling metadata only.

Rules:

- No tokens in `autospec-fleet.yml` or `fleet-node.yml`.
- Use `gh` and git credential helpers for auth.
- Never clone raw secrets beyond what the target repo checkout already contains.
- Do not log environment variables.
- Treat control repo config as code: validate before use and reject unknown
  top-level keys unless schema explicitly allows them.
- Keep target repo `.autospec/autospec.yml` as the source of project-specific
  test/deploy behavior.

## Testing

Local deterministic tests:

- URL normalization for HTTPS, SSH, and `.git` suffix forms.
- Config schema validation for required fields, unknown profile references, and
  disabled repos.
- Workspace slug mapping from `owner/repo` to `owner__repo`.
- Dry-run command generation for clone, fetch, and `/autospec-run`.
- Node profile filtering: a node only schedules repos whose profile it can run.
- Parallel cap: fleet never schedules more repos than both config and node allow.
- Status aggregation from mocked `list-ready-issues.sh`, `gh issue list`, and
  `gh pr list` JSON.
- Stop propagation writes or forwards the existing autospec stop sentinel.
- Install matrix includes `autospec-fleet` across Claude, OpenCode, and Codex.

Opt-in GitHub integration test:

- Create a temporary fleet workspace.
- Point it at two throwaway repos with pre-filed `auto-implement` issues.
- Run `autospec-fleet status` and `autospec-fleet run --dry-run`.
- Assert both repos are discovered and worker commands include distinct
  `fleet:<node>:<repo>` worker IDs.
- Delete the throwaway repos.

Verification commands:

```bash
autospec validate
bats tests/unit/test_autospec_fleet_config.bats
bats tests/unit/test_autospec_fleet_scheduler.bats
```

## Decomposition

Recommended implementation issues:

1. Skill scaffold, installer wiring, README, and lock-step prompt files.
2. Fleet config schema plus `fleet-config-lint.sh`.
3. Repo URL normalization, clone/sync workspace helper, and dry-run output.
4. Node-local config loading and profile/capacity matching.
5. Scheduler wrapper that invokes existing `/autospec-run` per repo.
6. Aggregated `status` and `stop` commands.
7. Documentation updates in README, user manual, API reference, and operations
   runbook.
8. Integration smoke test with mocked or throwaway GitHub repos.

## Decisions

| Question | Decision | Rationale |
| --- | --- | --- |
| New skill or extend `/autospec-run`? | New skill `autospec-fleet`. | Fleet is workspace and cluster orchestration; `/autospec-run` should remain one-repo implementation. |
| New central database? | No for v1. | Git config plus GitHub issue state is enough and easier to operate. |
| Config distribution? | Git control repo plus node-local file. | Gives review, rollback, and audit without exposing node capacity or secrets. |
| Issue-level scheduling in fleet? | No. | Existing `/autospec-run` already owns safe per-issue claims and conflict handling. |
| Empty-directory support? | Yes. | `init` and `sync` must work outside a target repo. |

## Open Follow-Ups

- Whether to add first-class launchd/systemd templates in v1 or defer them to a
  follow-up operations issue.
- Whether fleet should auto-run `/autospec-classify` for `needs-classify` issues
  or only report them in `status`.
- Whether the control repo should support repo groups such as `nightly`,
  `weekend`, or `local-only` in v1.
