# autospec end-of-run gap remediation loop — design

- **Date:** 2026-05-24
- **Status:** Approved in brainstorming; pending implementation plan
- **Author:** berlinguyinca (via Claude Code)
- **Supersedes:** the report-only `## Post-batch audit (autospec-review interlock)` section of `skills/autospec-run/SKILL.md`

## Goal

After an autospec run drains its `auto-implement` queue, automatically (1) review the shipped work for gaps across broad dimensions, (2) filter false positives, (3) file every surviving gap through the `needs-classify` safety-admission lifecycle, and (4) re-run the implementation loop for admitted gaps — bounded by a configurable round cap. This replaces the current end-of-run behavior, which only *reports* gap counts and never closes them.

## Motivation

The current interlock runs `/autospec-review --since "${BATCH_START_DATE}"` at end-of-run but only posts a gap-count comment; nothing acts on it. In the memory-consumers run (2026-05-24, issues #510–516) a *manual* deep review caught a real cross-platform correctness bug — `cross-repo-search.sh` built a grep pattern with a trailing `\|` that matches every line on GNU grep but errors-to-empty on BSD grep (gap **G1**) — plus a test blind spot (**G2**) and a diary separator ambiguity (**G3**). Pure spec-coverage review would have missed all three; they are implementation-quality gaps. Institutionalizing a broad, self-healing gap phase turns that manual catch into an automatic one.

## Decisions (resolved in brainstorming)

| # | Decision | Choice |
|---|----------|--------|
| D1 | What "address" means | Auto-file every surviving gap as `needs-classify`, admit it through the Rust safety gate, and re-loop admitted work. |
| D2 | Loop bound | `AUTOSPEC_GAP_MAX_ROUNDS`, default **2**. Early-exit on convergence (a round that files 0 new gaps). |
| D3 | Review scope | **Broad** — spec-coverage + correctness + test-quality + integration-wiring + docs — followed by an evaluate-findings/critic false-positive filter before filing. |
| D4 | Severity policy | File **all** severities (incl. docs/warn), **but dedupe** against open issues + active `docs:drift` self-heal so nothing double-files. |
| D5 | Architecture | Extend `/autospec-review` with a remediation mode (broad + filter + machine-readable gap emission); `/autospec-run` drives the bounded loop. (Approach A.) |
| D6 | Entry points | Applies to all paths that end in an autospec run: `/autospec`, `/autospec-define`→`/autospec-run`, and `/autospec-run` direct. The phase lives in autospec-run's post-batch section, which every path reaches. |

## Architecture

### Components

1. **`/autospec-review` remediation mode** (`--remediation`, writes gaps to `--emit-gaps <path>`):
   - Runs the existing spec-coverage review **plus** broad-dimension passes (correctness, test-quality, integration-wiring, docs-drift). Dispatched at **Tier A (opus)** per the monitor-tier lesson (`feedback_monitor_silent_exit.md`): the review is inline analysis and needs opus turn-stamina.
   - Pipes candidate findings through an **evaluate-findings/critic filter** (D3) to drop false positives before anything is filed.
   - Emits a machine-readable JSON gap list to `~/.autospec/gaps-<run-id>.json`:
     ```json
     [{"gap_id":"G1","dimension":"correctness","severity":"medium",
       "file":"skills/autospec-shared/scripts/cross-repo-search.sh","line":77,
       "title":"...","body":"...","dedupe_key":"cross-repo-search-trailing-pipe"}]
     ```

2. **`gap-remediation-loop.sh`** (deterministic driver, `skills/autospec-shared/scripts/`):
   - Reads the emitted gap JSON.
   - **Dedupes** (D4) against (a) open issues by `dedupe_key`/title-hash and (b) issues carrying an active `docs:drift` self-heal label.
   - Files survivors via `gh issue create --label needs-classify,gap-remediation,priority:high,origin:self`, then `/autospec-classify` adds quality/model-fit metadata and delegates admission to the Rust safety gate.
   - Tracks round state in `~/.autospec/gap-round-state.json`; enforces `AUTOSPEC_GAP_MAX_ROUNDS` (D2); reports convergence vs cap-hit.
   - Pure bash, `set +e` best-effort discipline (per `feedback_bash_*` memory); no RETURN-trap cleanup.

3. **`/autospec-run` Phase 5.5** (new SKILL.md section replacing the post-batch audit):
   ```
   round = 1;  MAX = ${AUTOSPEC_GAP_MAX_ROUNDS:-2}
   while round <= MAX:
     gaps = autospec-review --remediation --since BATCH_START --emit-gaps ~/.autospec/gaps-<run-id>.json
     survivors = gap-remediation-loop.sh --file --dedupe   # files needs-classify + returns count
     if survivors == 0: break          # converged
     autospec-classify newly-filed gap-remediation issues  # Rust-backed admission
     run Phase-4 monitor (opus, batch=1) until gap-remediation issues drain
     round++
   report (closed this phase + filter-suppressed + still-open-after-cap) → Phase 6 final report
   ```
   - **Skip controls unchanged:** `~/.autospec/no-review.flag`, `--no-postreview` skip the whole phase.

### Data flow

```
queue drains (ALL_DONE)
  → autospec-review --remediation  (broad review + filter, Tier A)  → gaps-<run-id>.json
  → gap-remediation-loop.sh        (dedupe + file needs-classify,gap-remediation issues)
  → autospec-classify              (quality/model-fit + Rust safety admission)
  → Phase-4 monitor                (opus, batch=1, drains admitted gap issues)
  → re-review (next round)         → converge (0 new) OR hit MAX
  → Phase 6 final report
```

### Termination / anti-loop guarantees

- **`gap-remediation` label** makes remediation issues recognizable so a later round does not re-flag freshly-fixed work as a new gap.
- **`dedupe_key`** prevents re-filing the same gap across rounds.
- **Convergence:** a round that files 0 new survivors ends the loop immediately.
- **Hard cap:** `AUTOSPEC_GAP_MAX_ROUNDS` (default 2); on cap-hit with gaps remaining, the loop stops and surfaces the remainder to the operator — it never spins.

## Error handling

- **Review subagent failure** → log a warning and fall back to today's report-only behavior; never block run completion (consistent with "post-review failures log a warning but do not fail the run").
- **`gh issue create` failure** → retry once, then emit the gap textually into the final report instead of silently dropping it.
- **Monitor stall on a gap issue** → existing failure isolation + relaunch discipline (opus, batch=1) applies unchanged.
- **Malformed/empty gap JSON** → driver treats as 0 survivors (converged); logs a warning.

## Testing

- **`gap-remediation-loop.sh` bats:** dedupe vs open issue; dedupe vs active `docs:drift`; round-cap enforcement; convergence early-exit; skip-flag honored; malformed/empty JSON handled.
- **`/autospec-review` remediation-mode bats:** emits valid gap JSON schema; the evaluate-findings filter drops a seeded false-positive; a seeded broad-dimension defect surfaces as a gap.
- **Lockstep:** `autospec validate` gains named-section checks for the new SKILL.md sections across both the `autospec-run` and `autospec-review` trios (per `feedback_validate_sh_lockstep_checks.md`).

## Out of scope

- Changing the per-issue implementer/reviewer inner loop.
- Mutation testing (tracked separately, #420).
- Tuning review heuristics for non-autospec target repos.

## Named consumer / ROI

Consumer: autospec-run operators. Immediate payoff: this phase would have auto-caught the G1-class cross-platform bug in the run that motivated it, with no manual review. No new top-level skill is introduced — the capability extends the existing `/autospec-review` (passes the ROI/named-consumer check in `feedback_roi_check_new_components.md`).
