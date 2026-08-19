# autospec-resume

Detect an interrupted **autospec** run from durable run-state + heartbeats on a
*fresh* start (host crash, session crash, terminal death) and **auto-continue**
it — without adding a second lock, without stealing a genuinely-live worker on
another host, without deleting un-pushed work, and capped at
`AUTOSPEC_RESUME_MAX_ATTEMPTS` (default 3) consecutive attempts.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-resume/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-resume/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-resume/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-resume/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-resume
./install.sh --harness all
```

## Flags

| Flag | Behaviour |
|---|---|
| (none) | Scan `in-progress-by-bot` and open `auto-implement` issues; recover exact stale-startup claims through Rust, then relaunch via the durably-captured command (clean-restart off `origin/main`). |
| `--resume-partial` | Additionally re-attach `/tmp/wt-<branch>` only when `heartbeat.host == $(hostname)`; cross-host or missing host clean-restarts. |
| `--repo <owner/name>` | Target a specific repo (used by the boot supervisor iterating the registry). |
| `--dry-run` | Print the recovery and relaunch actions that would run; invoke neither action; change nothing; exit 0. |

## How it stays safe

- **No second lock.** Resume relaunches the *run*; the relaunched monitor claims
  each issue through the **existing** GitHub CAS run-state lock. Two concurrent
  resumes converge to exactly one claim.
- **Crash-vs-live.** Eligibility is computed from run-state **server**
  `updated_at` (never a local clock): `step=claimed && age>=300` OR `age>=10800`.
  A fresh claimed issue is never stolen.
- **Stranded startup recovery.** The scan includes both relevant labels, but an
  `auto-implement` issue is eligible only for the exact stale `claimed` /
  `heartbeat-pending:none` sentinel. Rust owns the recovery CAS through
  `autospec claim state recover-stale-startup`, and only an exact matching
  `recovered: true` result becomes a relaunch candidate.
- **Read-only dry-run.** `--dry-run` reports both prospective actions before
  exiting and invokes neither recovery nor relaunch.
- **Cross-host.** `--resume-partial` only re-attaches a worktree whose heartbeat
  `host` matches `$(hostname)`; otherwise it clean-restarts off `origin/main`.
- **Durable relaunch command.** `/autospec-run` records the relaunch command in
  `~/.autospec/active-runs/<repo-slug>.json` at monitor launch, so it survives a
  reboot even though the session env var does not.
- **Attempt cap.** Bounded at `AUTOSPEC_RESUME_MAX_ATTEMPTS` (default 3); the
  counter resets when any issue reaches `merged`.

## Components

- `scripts/resume-scan.sh` — the decision engine (pre-conditions, crash-vs-live,
  cross-host gate, attempt cap; relaunches via the registry command).
- `scripts/resume-attempts.sh` — durable consecutive-attempt counter
  (`~/.autospec/resume-attempts/<repo-slug>.json`).
- `../../scripts/autospec-run-registry.sh` — durable run registry
  (`~/.autospec/active-runs/<repo-slug>.json`), written by `/autospec-run`.

## Spec

`docs/specs/2026-06-03-crash-resume-design.md`
