# Autospec Review — Design Spec

**Date**: 2026-05-06
**Repo**: github.com/berlinguyinca/autospec
**Status**: Approved (Phase 2 brainstorm complete; ready to decompose into issues)

## 1. Goals

Closed `auto-implement` issues regularly under-deliver against their parent
design specs. The four observed failure modes:

1. **Forgotten features** — spec AC items that never made it into any issue.
2. **Lying-closed state** — issue closed (PR merged) but the artifact it
   claimed to deliver is absent or empty in the repo.
3. **Half-finished merges** — issue closed but its body still has unchecked
   AC boxes, or the verification tests it cites don't exist or are skipped.
4. **Scope drift** — whole spec sections that never decomposed into issues.

`autospec-review` adds a 9th subskill that audits each design spec against
its open + closed issues, writes structured gaps to a CSV ledger, and routes
the gaps back through `/autospec-split` as `[REGRESSION]` issues with
`priority:high` + `regression` labels. The skill closes the feedback loop
between Phase 2's spec and Phase 4's actually-merged code.

User intent (verbatim, 2026-05-06):
> establish a feedback loop and do this in the future automatically

The skill is built skill-first and then run on chem-evidence as its first
real execution. The chem-evidence run is the validation: 20 design specs,
~864 issues (~40 open auto-implement, ~397+ closed), 8-skill autospec suite
already on `main`.

**Non-goals** (Phase 2 explicitly):

- Replacing the Implementation Guardian (kept; runs upstream of LGTM in
  every Phase 4 inner loop today).
- Auditing the autospec repo itself first (chicken-and-egg — autospec
  doesn't have the same per-spec issue topology).
- Cross-repo audits in v1 (one repo at a time; chem-evidence first).
- A scripted-only audit engine (v1 is subagent fan-out; deterministic
  pre-scan is a v2 candidate).
- Auto-merging regression PRs without admin approval (existing
  admin-merge flow stays unchanged).

## 2. Architecture

```
operator runs /autospec-review (or autospec-run auto-fires post-batch)
  │
  ▼
scripts/autospec_review_audit.py
  ├── discovers spec files (docs/specs/**, docs/superpowers/specs/**)
  ├── builds spec→issue linkage matrix (regex + gh search)
  └── emits per-spec audit input (yaml)
  │
  ▼
fan-out: 1 Tier-A subagent per spec (batched 5 at a time)
  │   each returns:  { spec_path, gaps[{gap_id, gap_type, severity, ...}] }
  ▼
merge → reports/autospec-review/<date>-<run_id>.csv  (per-run)
        reports/autospec-review/gaps.csv             (append-only ledger)
  │
  ▼ (per source spec with open gaps)
render templates/regression-spec.md.tmpl
  → docs/specs/<date>-<orig>-regressions.md   (in TARGET repo)
  │
  ▼
Tier-A reviewer subagent (NEW, §7a) reviews each regression spec
  → appends ### Reviewer notes section
  → may flip CSV rows to status=false_positive
  │
  ▼
git commit + branch autospec-review/<run_id>
  │
  ▼
invoke /autospec-split <regression-spec.md>
  → child issues filed; skill post-processes:
     • prepends [REGRESSION] to titles
     • adds labels priority:high + regression + topic-label
     • writes remediation_issue back to CSV; flips status=filed
  │
  ▼
/autospec-run picks up the issues:
  • queue sort: priority:high before non-priority regardless of age
  • on issues with label `regression`: LGTM reviewer escalated Tier B → Tier A
  • 2-pass meta-review: pass 2 asks "would Guardian/LGTM have caught the
    original gap? add the missing checklist items"
  • Implementation Guardian unchanged (already Tier A; upstream of LGTM)
  • lessons appended to reports/autospec-review/reviewer-lessons.md
```

```
autospec repo layout (additions)
─────────────────────────────────
skills/autospec-review/
  SKILL.md                              NEW — Claude Code body
  opencode/agent.md                     NEW — lock-step variant
  codex/prompt.md                       NEW — lock-step variant
  references/
    gap-taxonomy.md                     NEW — 4 detectors + severity rules
    csv-schema.md                       NEW — column dictionary
    subagent-contract.md                NEW — JSON shape per audit subagent
    reviewer-prompt.md                  NEW — verbatim §7a reviewer prompt
  templates/
    regression-spec.md.tmpl             NEW — one-regression-spec-per-source
  README.md                             NEW — human docs
  install.sh / uninstall.sh             NEW — per-skill installers (existing pattern)

scripts/
  autospec_review_audit.py              NEW — deterministic discovery + linkage
  validate.sh                           EXTEND — assert new lock-step + tier directives

skills/autospec-run/
  SKILL.md                              MODIFY — queue priority sort; Tier-A LGTM
                                        escalation + 2-pass on regression labels;
                                        post-batch /autospec-review trigger
  codex/prompt.md                       MODIFY (lock-step)
  opencode/agent.md                     MODIFY (lock-step)

tests/
  test_autospec_review.py               NEW — script unit tests
  test_phase4_regression_review.bats    NEW — byte-identity checks for new
                                        autospec-run regression-review block

SKILLS.md                               MODIFY — add autospec-review row
README.md                               MODIFY — mention 9th skill in suite
```

```
target repo layout (skill outputs at runtime)
─────────────────────────────────────────────
reports/autospec-review/
  <date>-<run_id>.csv          per-run snapshot
  gaps.csv                     append-only ledger (16 cols, status lifecycle)
  runs.md                      one entry per run, newest first
  reviewer-lessons.md          accumulating Tier-A LGTM lessons learned

docs/specs/
  <date>-<orig-slug>-regressions.md
                               one regression spec per source spec

git
  branch autospec-review/<run_id>
                               carries regression specs + (optional) /autospec-split outputs
```

## 3. Gap taxonomy and detection rules

Codified in `skills/autospec-review/references/gap-taxonomy.md`. The audit
subagent receives this file as part of its prompt.

| Gap type              | Detection signal                                                                         | Default severity |
|-----------------------|------------------------------------------------------------------------------------------|------------------|
| `ac_no_issue`         | Spec AC bullet has no semantic match in any linked issue's AC.                            | `major`          |
| `closed_missing_code` | Linked issue is `closed`, body cites a file/function/test/label, but `rg`/`gh api` confirms it's absent or empty. | `blocker`        |
| `closed_unchecked_ac` | Linked issue is `closed` but body has `- [ ]` boxes, OR `Verification:` cites tests that don't exist or are decorated `@pytest.mark.skip` (then escalate to `blocker`). | `major` (`blocker` if skipped) |
| `section_no_coverage` | Spec has top-level `## ` section whose slug matches no linked issue's title or labels.    | `minor`          |

The audit subagent does the **semantic match**; the script only collects
the candidate set. Subagent may downgrade defaults with reasoning recorded
in the `notes` column, or upgrade with reasoning. Severity flips are
visible in the ledger.

## 4. Audit engine — subagent contract

Codified in `references/subagent-contract.md`.

**Input** (yaml block in subagent prompt):

```yaml
spec_path: docs/specs/2026-04-30-source-completeness-remediation-design.md
spec_text: <full body, ≤ 80k tokens>
linked_issues:
  - { num: 472, state: closed, title: "...", body: "...", labels: [...], pr: 488 }
  - { num: 488, state: closed, title: "...", body: "...", labels: [...], pr: 491 }
  - ...
repo_root: /abs/path/to/target/repo
```

**Output** (JSON, machine-validated by the script):

```json
{
  "spec_path": "docs/specs/2026-04-30-source-completeness-remediation-design.md",
  "gaps": [
    {
      "gap_type": "closed_missing_code",
      "severity": "blocker",
      "title": "NLM source schema migration not present",
      "spec_anchor": "## 4.2 NLM source schema",
      "evidence": "Issue #488 (merged in PR #491) cites src/chem_evidence/sources/nlm/schema.py — file is absent. spec text quotes...",
      "suspected_issues": ["#472", "#488"],
      "remediation_hint": "Land src/chem_evidence/sources/nlm/schema.py with the columns enumerated in spec §4.2; add tests/test_nlm_schema.py covering validation."
    }
  ],
  "no_gaps_confidence": 0.0
}
```

Script validates JSON shape, computes `gap_id = sha1(spec_path + spec_anchor
+ gap_type)[:10]`, and rejects subagent output that fails schema (re-runs
once with the schema error in the prompt).

**Tier:** Tier A (top model + ultrathink). Same class as Phase 3.5 review
in autospec-define and autospec-classify per-issue review.

**Concurrency:** Batches of 5 subagents. With 20 specs, 4 batches.
Configurable via env `AUTOSPEC_REVIEW_BATCH_SIZE`.

## 5. CSV schema and lifecycle

Codified in `references/csv-schema.md`. Two files, identical schema:

- **Per-run snapshot:** `reports/autospec-review/<YYYY-MM-DD>-<run_id>.csv`
- **Append-only ledger:** `reports/autospec-review/gaps.csv`

`run_id` = `<UTC ISO compact>-<short_git_sha>` (e.g. `20260506T1430Z-6c2e3a4`).

| # | Column | Type | Description |
|---|--------|------|-------------|
| 1 | `gap_id` | string | `sha1(spec_path + spec_anchor + gap_type)[:10]` — primary key |
| 2 | `run_id` | string | `<UTC>-<sha>` of the audit run |
| 3 | `audit_date` | ISO date | yyyy-mm-dd |
| 4 | `repo` | string | `owner/name` |
| 5 | `spec_path` | string | repo-relative |
| 6 | `spec_topic` | string | filename slug (date-prefix removed) |
| 7 | `gap_type` | enum | `ac_no_issue` \| `closed_missing_code` \| `closed_unchecked_ac` \| `section_no_coverage` |
| 8 | `severity` | enum | `blocker` \| `major` \| `minor` |
| 9 | `title` | string | short imperative |
| 10 | `spec_anchor` | string | section heading the gap is anchored to |
| 11 | `evidence` | string | quote or grep result, ≤ 500 chars, newlines→`\n` |
| 12 | `suspected_issues` | string | space-separated `#nnn` list |
| 13 | `remediation_issue` | string | `#nnn` once filed; empty initially |
| 14 | `remediation_pr` | string | `#nnn` once /autospec-run merges it |
| 15 | `status` | enum | `open` \| `filed` \| `fixed` \| `wontfix` \| `false_positive` |
| 16 | `notes` | string | free text; user-editable; preserved across runs |

**Lifecycle rules:**

1. Initial write — every new gap → `status=open`.
2. After `/autospec-split` returns issue numbers — skill rewrites
   `remediation_issue` and flips `status=filed`.
3. Future audit runs key on `gap_id`. If a `status=filed` row's evidence
   no longer reproduces (artifact is now present, AC checked, etc.), skill
   flips to `status=fixed` **before** generating the run's regression spec
   so we don't refile fixed gaps.
4. Manual `status=wontfix` and `status=false_positive` are preserved
   across runs by `gap_id`. Their rows are excluded from regression-spec
   generation but still appear in the per-run CSV for audit trail.
5. A regression issue closed without merging on a future run → flip back
   to `status=open` with a note containing the closed issue number.

## 6. Remediation flow

`templates/regression-spec.md.tmpl`:

```markdown
---
date: {{audit_date}}
parent_spec: {{spec_path}}
audit_run_id: {{run_id}}
priority: high
---
# [REGRESSION] {{spec_topic}} — audit gaps {{audit_date}}

This spec enumerates gaps found by autospec-review on {{audit_date}}
against the parent spec. Each section corresponds to one row in
`reports/autospec-review/gaps.csv` (`gap_id` shown).

## Background

{{parent_spec_summary}}   <!-- 2-3 lines, generated by audit subagent -->

## Gaps to remediate

{{#each gaps}}
### Gap {{gap_id}} — `{{gap_type}}` — {{severity}}

**Parent spec anchor:** {{spec_anchor}}
**Suspected closed issues:** {{suspected_issues}}
**Evidence:**
> {{evidence}}

**What needs to ship:** {{remediation_hint}}

**Acceptance criteria:**
- [ ] {{ac_bullets_from_taxonomy}}
- [ ] Verification: {{verification_target}}

{{/each}}

## Verification

- [ ] Test suite for affected modules passes (commands generated from spec_topic)
- [ ] Each gap_id's evidence no longer reproduces (verified via re-grep)
- [ ] CSV row(s) flip from `status=filed` to `status=fixed` on next audit
```

**Step-by-step:**

1. Group CSV rows by `spec_path`; skip specs with zero `status=open` rows.
2. Render template per source spec to a working file (not yet committed).
3. **Tier-A reviewer subagent** (§7a below) reviews each rendered
   regression spec. It may flip rows to `status=false_positive`; those
   gaps are stripped from the spec before commit. Reviewer notes are
   appended as a `### Reviewer notes` section in the same file.
4. Create branch `autospec-review/<run_id>` in the target repo and
   commit the reviewed regression spec(s).
5. Invoke `/autospec-split docs/specs/<date>-<orig>-regressions.md`.
6. Post-process the issue numbers it returns:
   - Prepend `[REGRESSION] ` to title (idempotent).
   - Apply labels `priority:high`, `regression`, plus the parent spec's
     primary topic label (e.g. `usage-hierarchy`).
   - Append a footer to body: `> Generated by autospec-review run
     <run_id>. See gap_id <id> in reports/autospec-review/gaps.csv.`
   - Write `remediation_issue=#nnn` back to ledger; flip `status=filed`.

**Likely follow-up PR:** `/autospec-split` today inserts a "Pre-work N.0"
+ "Review N.review" sandwich (per `feedback_bracketed_issue_pattern`).
That structure is over-kill for regression issues, which already cite
specific fixes. autospec-split needs a small flag (e.g.
`--no-bracketing`) the skill passes on regression specs. Track as
follow-up issue created by autospec-review's first run.

## 7. Reviewer escalation on regression issues

This is the part that makes the feedback loop *learn*. Two layers:

### 7a. autospec-review's own reviewer pass (NEW subagent)

Before `/autospec-split` runs, dispatch one Tier-A subagent **per
regression spec** (not per gap). Verbatim prompt in
`references/reviewer-prompt.md`. The reviewer is told:

> Review this regression spec and the parent spec it points to. Ask:
> *do the gap descriptions actually match what the parent spec required?*
> Flag any gap that looks like a false positive, an over-broad ask, or a
> YAGNI violation. Suggest tightening the AC bullets if they invite
> scope creep. Output a `### Reviewer notes` section appended to the
> regression spec, plus optional `false_positive` flips for `gap_id`s
> you reject.

The reviewer's flips are merged into the CSV before /autospec-split runs.
This is the false-positive guard between gap detection and issue creation.

### 7b. autospec-run's LGTM reviewer escalation on regression issues

Today autospec-run's Phase 4 inner loop runs:
- **Implementation Guardian** (Tier A, upstream of LGTM) — already
  shipped (PR #221+, `scripts/lint-implementation.sh` present). Catches
  hallucinated symbols, scope creep, code duplication, doc drift, missing
  test types, complexity inflation, security regressions.
- **LGTM reviewer** (Tier B) — judges PR diff for correctness.

For issues with label `regression` OR `priority:high`, autospec-run adds:

> If issue.labels contains `regression` or `priority:high`:
>   1. LGTM reviewer escalates from Tier B to **Tier A** (top model +
>      ultrathink).
>   2. Reviewer runs **two passes**.
>      Pass 1 — standard LGTM judgment.
>      Pass 2 — meta-review with prompt:
>      > Would the Implementation Guardian or this LGTM reviewer have
>      > caught the original gap during the first implementation? If yes,
>      > what review questions failed? Add them to your checklist now and
>      > re-review with the augmented checklist. Append the new checklist
>      > items to `reports/autospec-review/reviewer-lessons.md` (one entry
>      > per item, with parent gap_id and date).
>   3. Both passes must approve before merge.
>
> Implementation Guardian is unchanged (already Tier A).

`reviewer-lessons.md` is consulted by future Tier-A reviewer passes —
older lessons stay durable, newer lessons stack on top. The file is
append-only (manual edits OK; rewrites discouraged).

### Tier inventory after this PR

| Subagent | Tier | When |
|---|---|---|
| Per-spec gap auditor (§4) | A | Every audit run |
| Per-regression-spec reviewer (§7a) | A | Every audit run with gaps |
| Implementation Guardian (existing) | A | Every Phase 4 inner loop (unchanged) |
| LGTM reviewer on `regression` issues (§7b) | A + 2-pass | Conditional on label |
| LGTM reviewer on normal issues (existing) | B | Default (unchanged) |
| Phase 4 implementer (existing) | B | Default (unchanged) |

Cost is intentionally heavy on regression issues only. Justification: a
regression that ships again costs more than two Tier-A passes.

## 8. Triggers, CLI, and concurrency safety

### CLI

```
/autospec-review [--spec PATH]            # audit one spec only
                 [--profile NAME]         # ~/.autospec/model-profiles.yml
                 [--dry-run]              # CSV + regression specs only; skip /autospec-split
                 [--no-autoreview]        # skip §7a Tier-A reviewer pass
                 [--since DATE]           # only audit specs whose date prefix is on/after DATE
                 [--spec-glob PATTERN]    # override default spec discovery globs
```

### Auto trigger from autospec-run (default ON)

New phase appended to autospec-run's outer loop, after the last issue in
a batch closes/merges:

```
Phase 6 (NEW): Post-batch audit
  if exists(~/.autospec/no-review.flag):  skip
  if --no-postreview was passed:          skip
  invoke /autospec-review --since <batch_start_date>
  on gaps found:
    post a comment to autospec-run status thread with gap counts by spec
    do NOT block batch completion
```

Three guards on the auto-trigger:

1. `~/.autospec/no-review.flag` — global opt-out, mirrors existing
   `~/.autospec/stop.flag` pattern.
2. `--no-postreview` — per-invocation opt-out for autospec-run.
3. `--since` — bounds re-audit work to specs in the batch's window.

Manual `/autospec-review` does a full audit by default.

### Concurrency

`~/.autospec/review.lock` (PID + start time) prevents two concurrent
`autospec-review` runs. Stale-lock auto-clear if PID isn't running and
lock is > 1h old. autospec-run honors the same lock and waits up to
30 min before logging a warning and skipping its post-batch audit.

### Notification

On run completion (manual or auto):

- Console summary: `run_id`, gaps by type, gaps by severity, regression
  issues filed.
- Path to per-run CSV + ledger.
- Append run summary to `reports/autospec-review/runs.md` (newest first).
- If env `AUTOSPEC_REVIEW_NOTIFY` set, post the same summary to that
  webhook (Slack/Discord). Reuses existing OMC notification config if
  detected; otherwise no-ops.

### Idempotency

Re-running with the same code state yields identical CSV rows (same
`gap_id`s). Already-`filed` rows are not refiled. Regression spec
filenames include `run_id`, so re-runs don't clobber prior specs.

## 9. autospec-run modifications (lock-step)

Required edits to `skills/autospec-run/{SKILL.md, codex/prompt.md,
opencode/agent.md}`:

1. **Queue priority sort.** Today: oldest `auto-implement` issue first.
   New rule: `priority:high` issues sort before non-priority regardless
   of age. Within `priority:high`, order is still by age.
2. **Tier-A LGTM escalation block.** Conditional on label `regression`
   OR `priority:high`: LGTM reviewer at Tier A; runs 2 passes; pass 2
   appends to `reports/autospec-review/reviewer-lessons.md`.
3. **Post-batch audit phase (Phase 6).** As described in §8.

`autospec validate` extended with:

- `check_autospec_run_priority_sort_lockstep` — byte-identity across
  the trio for the new sort block.
- `check_autospec_run_regression_review_lockstep` — same for the
  Tier-A escalation block.
- `check_autospec_review_skill_present` — validates the new skill
  trio's existence and lock-step.
- `check_autospec_review_tier_a_directives` — asserts both audit
  subagent and §7a reviewer subagent dispatches carry literal
  `Tier A (spec work)` annotations.

## 10. First-run plan on chem-evidence

Validation that the skill works end-to-end.

**Phase A — Build the skill (autospec repo).**

Branch `feat/autospec-review` against `github.com/berlinguyinca/autospec`:

1. Scaffold `skills/autospec-review/` per §2.
2. Add `scripts/autospec_review_audit.py` + `tests/test_autospec_review.py`.
3. Modify `skills/autospec-run/` (§9) for priority sort + Tier-A
   escalation + Phase 6 trigger.
4. Update `SKILLS.md`, `README.md`.
5. Extend `autospec validate` with the four new checks.
6. Run `autospec validate` — must pass.
7. Open one PR (skill is small; bigger churn would warrant split).

**Phase B — First run on chem-evidence.**

After PR merges and `~/.claude/skills/autospec-review` syncs:

1. From chem-evidence cwd: `/autospec-review --dry-run`. Expect
   20 audit subagents in 4 batches of 5.
2. Hand-review the dry-run CSV — focus on `severity=blocker` rows.
   If >20% look spurious, tune `gap-taxonomy.md` and re-run dry.
3. Run `/autospec-review` for real. Files `[REGRESSION]` issues via
   `/autospec-split`, applies labels, updates CSV `status=filed`.
4. Run `/autospec-run --profile <default>`. autospec-run picks up
   `priority:high` regression issues first; Tier-A LGTM kicks in;
   reviewer-lessons accumulates.

**Health metrics on first run:**

- **Total gap count.** Healthy: 5–50. <5 means audit too lax (tune up);
  >50 means too noisy (tune down or split with `--since`).
- **Severity distribution.** Too many `blocker`s → over-flagging;
  too few → rules too generous.
- **Status=fixed rate** (measured by re-audit weekly). Healthy:
  70–90% of `filed` rows fix cleanly within ~2 weeks. Remainder are
  hard cases (manual triage) or false positives (mark `wontfix`).
- **`reviewer-lessons.md` growth.** By run #3 the lessons file should
  be informing review prompts rather than starting from scratch.

**Risks for first run:**

- Tier-A subagent cost spike (20 audits + ≤20 reviewer passes + Tier-A
  LGTM on every regression issue). Mitigated by `--dry-run` gate +
  `--since` filter.
- A buggy `gap-taxonomy.md` could file dozens of bad regression issues.
  Mitigated by `--dry-run` + §7a reviewer pass.
- `/autospec-split`'s default issue template may be over-structured for
  regression specs. Likely follow-up PR adding `--no-bracketing` flag.

## 11. Lock-step compliance

Per `CONTRIBUTING.md`, when modifying any skill the body must stay
identical across SKILL.md / opencode/agent.md / codex/prompt.md (only
frontmatter differs).

This PR creates one new skill (autospec-review) and modifies one existing
skill (autospec-run). Both must pass lock-step diff:

```bash
diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
     <(cat skills/autospec-review/codex/prompt.md)
diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
     <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/opencode/agent.md)

diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md) \
     <(cat skills/autospec-run/codex/prompt.md)
diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md) \
     <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/opencode/agent.md)
```

`autospec validate` must enforce both.

## 12. Open follow-ups (out of scope for this PR)

- `/autospec-split --no-bracketing` flag for regression specs.
- v2 audit engine: scripted pre-scan + targeted subagent verification
  (cheaper than v1 fan-out at steady state).
- Cross-repo audits.
- Reviewer-lessons consolidation skill (when the file grows past ~200
  entries, summarize / dedupe).
- Auditing the autospec repo itself (after chem-evidence run validates
  the schema).
