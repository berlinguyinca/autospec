# Guarded-merge domain fence design (issue #1732)

## Problem

The blast-radius classifier + `autonomous-guardrails.sh blast-radius` + the
`fenced_surfaces` registry already exist and are tested. But they are **not
wired to the point where autospec actually merges to `main`**:

- The conductor invokes `autonomous-premerge-gate.sh` with only `--repo`
  (no `--changed-files`/`--fenced-surfaces`), so the gate's blast-radius block
  never runs during a drain cycle.
- `/autospec-run` Phase-4 merges each PR with a bare
  `gh pr merge --admin --squash --delete-branch` — no per-diff fence at all.

The only merge-time fencing today is **predictive**, at *selection* time
(`autonomous-prioritize.sh` routes fenced candidates to `human_gate` using issue
metadata/text, not the real diff). A manual `/autospec-run` drain bypasses even
that. So a PR whose **actual diff** touches a fenced surface (e.g. a trading
repo's `crates/risk-engine/**`) can merge unfenced.

## Scope of this change

Add a **per-diff fence at the merge chokepoint** by reusing the existing
classifier. This is the strongest fence available **without CI**; it is
implementer-honored (soft). The genuinely unbypassable fence is branch
protection + a required status check (CI) — tracked separately.

## Design

`scripts/autospec-guarded-merge.sh` — a fused check-then-merge wrapper that
`/autospec-run` calls **instead of** a bare `gh pr merge --admin`:

1. Read the PR's actual changed files (`gh pr view --json files`). gh failure →
   **fail-closed** (exit 2, not merged).
2. Classify via `autonomous-guardrails.sh blast-radius` against the repo's
   `fenced_surfaces` registry (default resolution: `.autospec/fenced-surfaces.yml`
   → `.autospec/autospec.yml`; overridable with `--fenced-surfaces`).
3. Branch on the deterministic `DECISION:` line (not exit code alone) so a
   classifier error is distinguished from a real quarantine and fails closed:
   - `DECISION:quarantine` + no override label → apply `autospec:needs-human`,
     comment the fenced surfaces, print `blocked fenced_surface`, **exit 1
     (not merged)**.
   - `DECISION:quarantine` + `autospec:fenced-approved` override label present →
     proceed to merge (logged).
   - `DECISION:allow` (or empty diff) → merge.
4. Merge with the same `--admin --squash --delete-branch` (configurable via
   `--merge-args`).

Calling the wrapper instead of `gh pr merge` means "merge without the fence
check" requires deliberately bypassing the wrapper, not merely omitting a prose
step. `/autospec-run` `SKILL.md` and `prompts/phase4-implementer.md` are wired
to call it and to stop-with-`needs-human` on a quarantine (exit 1) or pause on a
fail-closed error (exit 2) rather than treating it as a merged success.

## Per-repo configuration

Each protected repo declares its fenced crates in `fenced_surfaces`. For
`berlinguyinca/autotrade` this is `crates/{risk-engine,execution-engine,
signal-engine,consensus-engine}/**` (money/risk/execution/strategy authoring).
Without a repo-specific registry the classifier's legacy default fences
`crates/*` broadly, so a real `fenced_surfaces` entry is required to keep
ordinary crate work (e.g. test-hardening) mergeable while fencing the sensitive
crates.

## Testing

`tests/autonomous/test_guarded_merge.bats` drives the REAL classifier against a
fixture registry via a stubbed `gh`: allow (non-fenced → merges), quarantine
(fenced, no override → blocked + `needs-human`, not merged), override (fenced +
label → merges), fail-closed (gh error → exit 2, not merged), empty diff, and
missing-arg invocation error.

## Non-goals

- CI / branch protection (the truly-hard, unbypassable fence) — separate work,
  depends on standing up external CI.
- Changing the predictive selection-time gate.
