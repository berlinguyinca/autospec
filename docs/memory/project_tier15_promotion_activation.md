---
name: project-tier15-promotion-activation
description: "How autospec-autonomous Tier 1.5 issue-promotion actually activates, and why the existing needs-classify backlog is unpromotable."
metadata:
  node_type: memory
  type: project
  originSessionId: ad29d98b-2c40-47a9-baf7-c4a47462e9db
---

autospec-autonomous's Tier 1.5 ("promote existing open issues into the auto-implement queue")
shipped for a long time as a **placeholder no-op** (`classify-model-fit.sh --help >/dev/null`) —
so a dry Tier-1 backlog fell straight through to Tier-2 explore discovery and NEVER drained
existing tracker issues, despite the SKILL.md implying it would.

Real promoter added 2026-07-09 via issue #1691 / PR #1692 (`scripts/autonomous-promote-open-issues.sh`),
wired as fallback in `scripts/lib/autospec-loop.sh`.

**To actually activate promotion it is DOUBLE-GATED — both required:**
- the conductor auto-detects the script and calls it with `--apply`, AND
- the operator must set env `AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY=1`.
Without the env var it is REPORT-ONLY (emits `{"dry":true,"filed":0,...}`, mutates nothing).
This is deliberate blast-radius protection: merging the feature does NOT make a live
admin-auto-merge conductor start auto-labeling.

**Selector is conservative:** only open `needs-classify` issues, body ≥80 chars, excluding
`auto-implement`/`epic`/`needs-autospec-template`/`blocked*`/`paused-by-user`/
`locked-by-autospec-processor`/`autospec:needs-human`/`wontfix`/`duplicate`.

**Gotcha — the existing backlog is unpromotable:** as of that date, ALL open `needs-classify`
issues ALSO carried `needs-autospec-template` (identical sets), so the promoter correctly
promotes ZERO. Those are template-incomplete stubs (missing `## Files to read first` /
`## Implementation scope`); they must be TEMPLATED first (real `/autospec-classify` /
`/autospec-define` work) before Tier 1.5 will ever pick them up. "Seed the backlog" is not
just a labeling step for these.

**Env caveats:** the launcher does NOT source `~/.autospec/env`, and `~/.autospec/scripts` is a
copy-target overwritten by the daily `bootstrap.sh --update` (NOT a git checkout) — so
persistent behavioral env must reach the conductor's launch environment / supervisor, and
script fixes must land in the repo (origin/main), never hand-edited under `~/.autospec/scripts`.

## Why the headless conductor convergence-parks with 0 work done (2026-07-09)

Root cause verified: the detached `autospec-autonomous.sh run-foreground` bash loop bridges the
**Tier-1 drain** to an LLM (`AUTOSPEC_RUN_CMD` → `autospec-autonomous-run-drain.sh` → `omx exec
'$autospec-run'`, launcher line ~941) but **never sets `AUTOSPEC_EXPLORE_CMD`**, so Tier-2/3/4
discovery runs as bare `bash autospec-explore.sh --once` with no LLM orchestrator. Explore's
researcher-proposal seam (`AUTOSPEC_SPECIALIST_PROPOSALS_*`) and its fail-closed autonomous
adversarial verify (Tier-B skeptic per proposal) need an orchestrator to dispatch subagents;
absent one, every proposal is refused → `dry=true filed=0` every cycle at ~0 tokens. All tiers
dry → `autonomous-waterfall.sh:237` emits a hard `park` (loop exits) instead of the spec-mandated
`idle-rescan` (R1/R5). Net: conductor exits in ~11 cycles having done nothing.

Two defects: (1) missing explore LLM bridge — needs an `omx exec '$autospec-explore --once'`
wrapper + `AUTOSPEC_EXPLORE_CMD` wiring, mirroring the drain; the loop parses explore stdout for
the contract `{"tier","proposals_seen","new_candidates","filed","dry","reason"}` (ground truth:
tests/autonomous/test_sandbox_routing.bats). (2) convergence hard-park instead of idle-rescan.
All three defects now FIXED + MERGED to main (2026-07-09): promoter PR #1692, idle-rescan
PR #1699, explore-bridge PR #1698 (adds `scripts/autospec-autonomous-explore-drain.sh` +
`AUTOSPEC_EXPLORE_CMD` wiring). **Caveat: fixes reach the running conductor only after
`~/.autospec/scripts` is refreshed** (daily self-update or `install.sh --update`) — a bare
relaunch on a stale install reproduces the convergence-park. After install, the LLM-bridged
explore actually spends real tokens per idle discovery cycle (was ~0 before). Design refs:
docs/specs/2026-06-25-autospec-autonomous-design.md + phase2/3.

**Launch gotcha (verified 2026-07-09):** `autospec-autonomous start --repo OWNER/REPO` alone
defaults `AUTOSPEC_REPO_DIR` to the launcher's install path `~/.autospec` (NOT a git repo, NOT
the source) even when launched from inside the checkout — so the omx-bridged explore/drain run
`--cd ~/.autospec` and analyze garbage. ALWAYS pass `--repo-dir /path/to/checkout` (e.g.
`--repo-dir /home/wohlgemuth/IdeaProjects/autospec`). Verify via
`~/.autospec/autonomous-operator/<scope>/launch.json` `.repo_dir` and the running process env.
FIXED in PR #1702 (merged, commit dbd8dcb): `start_detached` now defaults repo-dir to the launch
cwd's `git rev-parse --show-toplevel` and FAILS LOUD if the resolved dir isn't a git checkout —
so once `~/.autospec` is refreshed past dbd8dcb, launching from inside the checkout "just works"
and a non-repo target errors instead of silently analyzing `~/.autospec`. Until installed, still
pass `--repo-dir`.

## Explore adversarial-verify bridge (Option A, 2026-07-09)

Explore's fail-closed autonomous verify (`autospec-explore.sh` ~L916-973 degradation ladder)
had NO verifier wired in the detached conductor, so every idle explore cycle generated
proposals (`proposals_seen:92`) but filed ZERO (`reason:verify-unavailable-failclosed`).
Fix = wire `AUTOSPEC_EXPLORE_VERIFY_CMD` to an omx skeptic bridge.

**Two probes settled the design (both verified live 2026-07-09):**
- **Env propagation:** a launcher-exported env var survives launcher→`omx exec`→codex→`bash -lc`
  (probe: `env AUTOSPEC_PROBE=hello omx exec --cd /tmp --dangerously-bypass-approvals-and-sandbox '…echo $AUTOSPEC_PROBE'` → `hello`). So wiring the seam in `start_detached` (like `AUTOSPEC_RUN_CMD`/`AUTOSPEC_EXPLORE_CMD`) reaches `autospec-explore.sh` running INSIDE the codex session.
- **Nested omx works:** `omx exec` inside an `omx exec` session (codex-in-codex) runs cleanly
  with `--dangerously-bypass-approvals-and-sandbox` (~5s, auth propagates, no deadlock). So the
  verify seam — invoked as `bash -c "$AUTOSPEC_EXPLORE_VERIFY_CMD"` between explore's dedup/finalize
  passes — may itself shell to omx (nested) without re-architecting the #1698 explore bridge.

**Exact seam contract (authoritative: tests/explore/test_explore_orchestrator_verify.bats +
test_explore_e2e.bats):** verify cmd reads `$AUTOSPEC_EXPLORE_DEDUPED_IN` = `{"deduped":[{norm_title,title,evidence,estimated_complexity,confidence,…}]}`, writes `$AUTOSPEC_EXPLORE_VERDICTS_OUT` = `{norm_title:{"verdict":"survived"|"refuted","reason":…}}`. Success = exit 0 AND non-empty out file. Consumer refutes-by-default: missing entry or verdict≠"survived" ⇒ refuted; broken/absent verify ⇒ `verify_mode=no-op-unverified` + `code_health:explore_verify_noop` and files nothing (SAFE). So a skeptic that errors/emits garbage MUST fail-closed (non-zero/empty), never all-survive.

**Bridge shipped in two PRs (both merged 2026-07-10):** #1704 (`scripts/autospec-autonomous-verify-drain.sh` + launcher wiring at `autospec-autonomous.sh:955`, `start_foreground`) and #1707/#1708 (robust verdict extraction). #1704's mocked bats used a CLEAN fenced ```json block, so it shipped green but the real bridge wrote `{}` against live omx: real codex/omx output emits the verdict as RAW JSON buried in brace-heavy chatter (`${VAR:-default}` shell braces, `hook:` telemetry, the final assistant message printed TWICE, a `tokens used` block, a trailing `{"event":"turn_complete",...}` footer). The extractor took the FIRST parseable dict → a telemetry object with no key-overlap with known norm_titles → normalized to `{}` → exit 0 → consumer refuted-by-default → 0 filed while looking successful. Classic [[feedback-self-consistent-test-fixtures-mask-bugs]]: caught ONLY by running the installed script against REAL omx (the mocked test could not). Fix (#1708): prompt now wraps verdicts in `===AUTOSPEC_VERDICTS_BEGIN/END===` sentinels (primary extraction = last marker pair); fallback = pick the candidate dict with GREATEST key-overlap with `known` (not first-parseable); and N>0 proposals but ZERO overlapping verdicts ⇒ `exit 3` fail-closed OBSERVABLY (never silent `{}`+exit0). Verified end-to-end against real omx: destructive "delete all tests" → refuted, concrete resilience proposal → survived, populated map on disk.

**No conductor restart needed to pick up verify-drain fixes:** `AUTOSPEC_EXPLORE_VERIFY_CMD` points at the script PATH and explore invokes it fresh (`bash -c`) each Tier-4 cycle, so refreshing the on-disk copy (`install.sh --update`) is enough — the running conductor uses the new script on its next explore cycle. (A restart IS needed only to re-export a NEWLY-ADDED launcher env seam, e.g. #1704's initial wiring.)

Related: [[feedback-refine-then-run-workflow]]
