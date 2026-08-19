---
name: autospec-monitor
description: Observe a live autospec-autonomous conductor read-only by summarizing typed status, timeline, launch, heartbeat, tier, and discovery artifact surfaces.
mode: primary
---
# autospec-monitor

Use this skill when the operator asks for live observability into an `autospec-autonomous`
run: what conductor is running, whether it was launched in real mode or `--dry-run`,
which cycle/tier/action is current, how fresh the heartbeat is, what stop flag is set,
and what discovery artifacts were produced.

Autospec monitor is read-only by default. It must not mutate repository state or GitHub
state unless the operator explicitly asks it to file a monitor-related issue.

## Operating Rules

- Prefer the typed Rust command surfaces over raw log parsing.
- Use `autospec autonomous monitor --once` for a concise operator snapshot.
- Use `autospec autonomous status --json` and `autospec autonomous timeline --lines N`
  for follow-up detail; do not hand-parse logs unless those typed fields are missing.
- Always distinguish launch mode from tier result:
  - `run_mode=dry-run` means the conductor launch argv included `--dry-run`.
  - `tier_dry_result.dry=true` means that tier filed no candidates in that cycle;
    it does not by itself mean the conductor was launched with `--dry-run`.
- Accept and pass through `--repo OWNER/REPO` and `--repo-dir DIR` using the same
  conventions as `autospec-autonomous` / `autospec autonomous`.
- Stay useful when fields are partially empty by reporting `-`, `null`, or an empty
  artifact list rather than inventing state.
- GitHub issue writes require explicit operator confirmation and must be limited to
  monitor-related defects or enhancement requests.

## Command Shape

```bash
autospec autonomous monitor --once --repo OWNER/REPO --repo-dir DIR
```

For machine-readable output:

```bash
autospec autonomous monitor --once --json --repo OWNER/REPO --repo-dir DIR
```

For a broader timeline after the snapshot:

```bash
autospec autonomous status --json --repo OWNER/REPO --repo-dir DIR
autospec autonomous timeline --lines 80 --repo OWNER/REPO --repo-dir DIR
```

## What To Report

Summarize these fields when present:

- conductor identity: repo, PID, log path, launch argv, stop flag path/mode;
- launch mode: `real` versus `dry-run` from launch argv;
- current work: cycle, tier, action, state status, and heartbeat age;
- tier dry result: explain `dry=true` as a no-filed-candidates tier result, not as
  proof of launch `--dry-run` mode;
- discovery artifacts: `.autospec/explore-once-*` metadata, proposal counts,
  candidate/filed issue titles, and verification mode.

## Intent Mapping

| Operator asks | Do this |
| --- | --- |
| What is autospec-autonomous doing? | Run `autospec autonomous monitor --once --repo OWNER/REPO --repo-dir DIR`. |
| Is this a real run or dry-run? | Report `run_mode` from monitor output and quote the launch argv evidence. |
| Did Tier 2/4 dry mean dry-run? | Explain that tier `dry=true` means no filed candidates for that tier cycle. |
| What did discovery find? | Report `.autospec/explore-once-*` proposal/candidate counts and titles. |
| Fields are missing or empty. | Fall back to status/timeline/artifact summaries and mark unknown fields as `-` or `null`. |
| File an issue about monitor output. | Confirm the requested write, then file only that monitor-related issue. |
