# autospec end-of-run gap remediation loop — design

- **Date:** 2026-05-24
- **Status:** Approved in brainstorming; pending implementation plan
- **Author:** berlinguyinca (via Claude Code)
- **Supersedes:** the report-only `## Post-batch audit (autospec-review interlock)` section of `skills/autospec-run/SKILL.md`

## Goal

After an autospec run drains its `auto-implement` queue, automatically (1) review the shipped work for gaps across broad dimensions, (2) filter false positives, and (3) render every survivor into the full issue-quality template before staging it through the `needs-classify` lifecycle. Generated gaps do not bypass quality checks or Rust-backed safety admission; the existing autonomous Tier 1.5 path owns that later transition.

## Motivation

The current interlock runs `/autospec-review --since "${BATCH_START_DATE}"` at end-of-run but only posts a gap-count comment; nothing acts on it. In the memory-consumers run (2026-05-24, issues #510–516) a *manual* deep review caught a real cross-platform correctness bug — `cross-repo-search.sh` built a grep pattern with a trailing `\|` that matches every line on GNU grep but errors-to-empty on BSD grep (gap **G1**) — plus a test blind spot (**G2**) and a diary separator ambiguity (**G3**). Pure spec-coverage review would have missed all three; they are implementation-quality gaps. Institutionalizing a broad, self-healing gap phase turns that manual catch into an automatic one.

## Decisions (resolved in brainstorming)

| # | Decision | Choice |
|---|----------|--------|
| D1 | What "address" means | Auto-file every surviving gap as `needs-classify`; never admit or drain generated bodies inline. |
| D2 | Run bound | One staging pass per autospec run. The driver retains `AUTOSPEC_GAP_MAX_ROUNDS` as defensive state for direct/repeated invocation. |
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
   - Deterministically renders each survivor into the full issue template, requires `lint-issue.sh` to pass, and only then files it via `gh issue create --label needs-classify,gap-remediation,priority:high,origin:self`. Tier 1.5 later classifies and delegates admission to the Rust safety gate.
   - Tracks round state in `~/.autospec/gap-round-state.json`; enforces `AUTOSPEC_GAP_MAX_ROUNDS` (D2); reports convergence vs cap-hit.
   - Pure bash, `set +e` best-effort discipline (per `feedback_bash_*` memory); no RETURN-trap cleanup.

3. **`/autospec-run` Phase 5.5** (new SKILL.md section replacing the post-batch audit):
   ```
   gaps = autospec-review --remediation --since BATCH_START --emit-gaps ~/.autospec/gaps-<run-id>.json
   survivors = gap-remediation-loop.sh --file --dedupe   # files needs-classify + returns count
   if survivors == 0: report clean
   else: report staged gap-remediation issues
   autonomous Tier 1.5 later grooms/classifies/admit gaps
   ```
   - **Skip controls unchanged:** `~/.autospec/no-review.flag`, `--no-postreview` skip the whole phase.

### Data flow

```
queue drains (ALL_DONE)
  → autospec-review --remediation  (broad review + filter, Tier A)  → gaps-<run-id>.json
  → gap-remediation-loop.sh        (dedupe + render + quality-lint + file needs-classify issues)
  → Phase 6 final report           (staged gaps are visible, not claimed closed)
  → later Tier 1.5 cycle           (model-fit + Rust admission)
  → later Phase-4 monitor          (drains only admitted gap issues)
```

### Termination / anti-loop guarantees

- **`gap-remediation` label** makes remediation issues recognizable so a later run does not re-file the same open work.
- **`dedupe_key`** prevents re-filing the same gap across runs.
- **Convergence:** a pass that finds 0 new survivors reports clean immediately.
- **No inline drain:** a pass that stages survivors stops after reporting them, so it cannot spin on unadmitted work. The driver's `AUTOSPEC_GAP_MAX_ROUNDS` remains defense in depth for direct/repeated invocation.

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
