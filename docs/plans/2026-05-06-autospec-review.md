# Autospec Review — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `autospec-review` — a 9th autospec subskill that audits design specs vs open + closed issues, writes gaps to a CSV ledger, and routes regressions back through `/autospec-split` with `priority:high` + `regression` labels — plus the autospec-run modifications that make the feedback loop fire automatically.

**Architecture:** Subagent fan-out per spec (Tier A) → JSON gaps merged into a 16-column CSV ledger → one regression spec per source spec, reviewed by a Tier-A subagent, committed on `autospec-review/<run_id>` branch → `/autospec-split` decomposes into issues → `[REGRESSION]`-prefixed and labeled `priority:high` + `regression`. autospec-run sorts those issues to the front of its queue and escalates its LGTM reviewer Tier B → Tier A with a 2-pass meta-review that appends to `reviewer-lessons.md`.

**Tech Stack:** Bash (skill body, install.sh, validate.sh checks), Python 3.11+ (`scripts/autospec_review_audit.py`, pytest tests), `gh` CLI (issue queries, PR creation), `jq` (JSON parsing in shell), bats (existing test runner for skill body byte-identity).

**Spec reference:** `docs/specs/2026-05-06-autospec-review-design.md`

---

## File Structure

**New files (autospec repo):**

```
docs/plans/2026-05-06-autospec-review.md            # this plan
skills/autospec-review/
  SKILL.md                                          # Claude Code body (canonical)
  opencode/agent.md                                 # OpenCode variant (lock-step)
  codex/prompt.md                                   # Codex variant (lock-step)
  README.md                                         # human docs
  install.sh / uninstall.sh                         # per-skill installers
  references/
    gap-taxonomy.md                                 # 4 detectors + severity rules
    csv-schema.md                                   # column dictionary
    subagent-contract.md                            # JSON shape per audit subagent
    reviewer-prompt.md                              # verbatim §7a reviewer prompt
  templates/
    regression-spec.md.tmpl                         # one-regression-spec-per-source
scripts/
  autospec_review_audit.py                          # spec discovery, linkage matrix, CSV I/O
tests/
  test_autospec_review.py                           # pytest unit tests for the script
  test_autospec_review_skill_lockstep.bats          # skill body byte-identity
  test_autospec_run_regression_review_lockstep.bats # regression-review block byte-identity
  test_autospec_run_priority_sort_lockstep.bats     # priority-sort block byte-identity
```

**Modified files:**

```
skills/autospec-run/SKILL.md                        # +queue priority sort
                                                    # +Tier-A LGTM escalation block (2-pass)
                                                    # +Phase 6 post-batch trigger
skills/autospec-run/opencode/agent.md               # lock-step copy
skills/autospec-run/codex/prompt.md                 # lock-step copy
scripts/validate.sh                                 # +4 new check_* functions
SKILLS.md                                           # +autospec-review row
README.md                                           # +9th-skill mention in suite
```

**Responsibility boundaries:**
- `scripts/autospec_review_audit.py` — deterministic only. Spec discovery, GitHub linkage, CSV writes, JSON validation, gap_id hashing. No LLM calls. Easy to unit-test.
- `skills/autospec-review/SKILL.md` — orchestrator prompt. Drives subagent dispatch, decides when to escalate. References the script for I/O.
- `skills/autospec-run/SKILL.md` — minimal additions. Queue sort logic + conditional reviewer block + post-batch trigger. Does NOT know about CSV internals.
- `references/*.md` — pure documentation, loaded into subagent prompts at runtime.

---

## Branch & commit strategy

All work continues on existing branch `feat/autospec-review` (already pushed, PR #240 open). Commit per task — frequent commits, small diffs. Lock-step changes get committed as one unit per logical change (SKILL.md + opencode + codex together).

Pre-flight check before starting:
```bash
cd /Users/wohlgemuth/IdeaProjects/autospec
git rev-parse --abbrev-ref HEAD     # expect: feat/autospec-review
git status --short                   # expect: clean (only docs/superpowers/ artifacts/ untracked)
```

---

## Task 1: Add docs/plans/ directory and commit this plan

**Files:**
- Create: `docs/plans/2026-05-06-autospec-review.md` (this file)

- [ ] **Step 1: Stage and commit the plan**

```bash
cd /Users/wohlgemuth/IdeaProjects/autospec
git add docs/plans/2026-05-06-autospec-review.md
git commit -m "docs(plan): autospec-review implementation plan"
```

Expected: clean commit on `feat/autospec-review`.

---

## Task 2: Scaffold autospec-review skill directory structure (empty placeholder files)

**Files:**
- Create: `skills/autospec-review/SKILL.md` (header only)
- Create: `skills/autospec-review/opencode/agent.md` (header only)
- Create: `skills/autospec-review/codex/prompt.md` (empty)
- Create: `skills/autospec-review/README.md`
- Create: `skills/autospec-review/install.sh`
- Create: `skills/autospec-review/uninstall.sh`
- Create: `skills/autospec-review/references/.keep`
- Create: `skills/autospec-review/templates/.keep`

- [ ] **Step 1: Look at autospec-stop's install.sh as the simplest reference**

```bash
cat skills/autospec-stop/install.sh
cat skills/autospec-stop/uninstall.sh
```

- [ ] **Step 2: Create the directory tree**

```bash
mkdir -p skills/autospec-review/{references,templates,opencode,codex}
touch skills/autospec-review/references/.keep
touch skills/autospec-review/templates/.keep
```

- [ ] **Step 3: Create SKILL.md frontmatter only**

```markdown
---
name: autospec-review
description: Use when the user wants to audit design specs against open + closed issues to find gaps, file high-priority regression issues, and feed them back through autospec. Runs as `/autospec-review` (manual) or auto-fires after each autospec-run batch unless `~/.autospec/no-review.flag` exists.
---

<!-- BODY START -->
<!-- (filled in Task 13–15) -->
<!-- BODY END -->
```

- [ ] **Step 4: Create opencode/agent.md frontmatter only**

```markdown
---
description: Audit design specs against open + closed issues to find gaps, file high-priority regression issues, and feed them back through autospec. Runs as `/autospec-review` (manual) or auto-fires after each autospec-run batch unless `~/.autospec/no-review.flag` exists.
mode: primary
---

<!-- BODY START -->
<!-- BODY END -->
```

- [ ] **Step 5: Create codex/prompt.md (no frontmatter — leading blank line for diff parity)**

```

<!-- BODY START -->
<!-- BODY END -->
```

- [ ] **Step 6: Create install.sh based on autospec-stop's pattern**

```bash
#!/usr/bin/env bash
# Skill-local installer; delegates to top-level install.sh
set -euo pipefail
SKILL_NAME="autospec-review"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/../../install.sh" --skill "$SKILL_NAME" "$@"
```

`chmod +x skills/autospec-review/install.sh skills/autospec-review/uninstall.sh`

- [ ] **Step 7: Create uninstall.sh (mirror)**

```bash
#!/usr/bin/env bash
set -euo pipefail
SKILL_NAME="autospec-review"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/../../uninstall.sh" --skill "$SKILL_NAME" "$@"
```

- [ ] **Step 8: Create README.md stub**

```markdown
# autospec-review

Audits design specs against open + closed GitHub issues. Finds gaps,
writes them to `reports/autospec-review/gaps.csv`, and files
`[REGRESSION]` issues with `priority:high` + `regression` labels.

See `docs/specs/2026-05-06-autospec-review-design.md` in this repo for
full design.

## Usage

    /autospec-review [--spec PATH] [--profile NAME] [--dry-run]
                     [--no-autoreview] [--since DATE]

See SKILL.md for full flag semantics and triggering rules.
```

- [ ] **Step 9: Verify install.sh discovers the new skill**

```bash
bash install.sh --skill autospec-review --dry-run 2>&1 | tail -5
```
Expected: skill name resolves, install plan printed, no errors.

- [ ] **Step 10: Commit**

```bash
git add skills/autospec-review/
git commit -m "feat(autospec-review): scaffold skill directory and installers"
```

---

## Task 3: Write references/gap-taxonomy.md

**Files:**
- Create: `skills/autospec-review/references/gap-taxonomy.md`

- [ ] **Step 1: Write the taxonomy doc**

```markdown
# Gap taxonomy and detection rules

This document is loaded verbatim into the audit subagent prompt. Each
gap type has: a detection signal, examples (positive and negative), a
default severity, and rules for severity escalation.

## 1. `ac_no_issue` — forgotten features

**Signal:** Spec contains an Acceptance Criteria bullet (line matching
`^- \[[ x]\] ` under a `## Acceptance Criteria` heading) that has no
semantic match in any linked issue's AC list.

**Semantic match rule:** an issue AC is a match if it covers the same
required behaviour, even if phrasing differs. The bullet "POST /api/foo
returns 201 on success" matches "REST endpoint /api/foo creates a foo
record". Mismatches: "must support batch insert" does NOT match "supports
single insert" (scope difference).

**Default severity:** `major`.
**Escalate to `blocker`:** if the bullet contains MUST/REQUIRED words.

## 2. `closed_missing_code` — lying-closed state

**Signal:** A linked issue is `closed` (or its PR is merged), the issue
body explicitly cites a deliverable (file path, function name, test
path, label name, env var, CLI flag), but the deliverable is absent from
the repo's HEAD or is empty.

**Verification (subagent must run):**
- For file paths: `test -e <path>` AND `[ -s <path> ]`
- For function/symbol names: `rg -F "<name>" <repo_root>` returns ≥ 1 hit
- For test paths: `pytest --collect-only <path>` returns ≥ 1 test
- For labels: `gh label list | rg "^<name>"` returns 1
- For env vars: `rg -F "<NAME>" <repo_root>` returns ≥ 1 hit
- For CLI flags: `--help` output mentions the flag

**Default severity:** `blocker`. No down-grade.

## 3. `closed_unchecked_ac` — half-finished merges

**Signal:** A linked issue is `closed` but its body still contains
`- [ ]` (unchecked) checkboxes inside an AC list, OR the issue body's
`Verification:` section cites tests that don't exist, are empty, or are
decorated `@pytest.mark.skip`/`@pytest.mark.xfail`/`it.skip`/`it.todo`.

**Default severity:** `major`.
**Escalate to `blocker`:** if any cited test is skipped/xfail (the test
exists but is disabled — strongest signal of half-finished work).

## 4. `section_no_coverage` — scope drift

**Signal:** Spec has a top-level `## ` section (depth 2 heading) whose
slug matches no linked issue's title (case-insensitive substring) and no
linked issue's labels. Subsections (`### `) are not flagged here — only
top-level sections.

**Slug derivation:** lowercase the heading text, strip non-alphanumeric
characters, replace whitespace with `-`. Example: `## 4.2 NLM Source
Schema` → `nlm-source-schema`.

**Default severity:** `minor`.
**Escalate to `major`:** if the section heading contains words like
"Architecture", "Goals", "Deliverables", "Phases" (semantic weight).

## Severity downgrade rules (subagent reasoning)

The audit subagent may downgrade defaults — but must record the reason
in the `notes` column of its emitted gap. Acceptable downgrade reasons:

- "Section is purely background context, no deliverables" → minor
- "AC bullet is a meta-AC about review, covered by existing review process"
  → minor
- "Symbol is renamed in code; equivalent function exists at <path>"
  → flag as `closed_missing_code` minor instead of blocker

Subagent may NOT downgrade `closed_missing_code` to non-blocker without
citing a renamed equivalent.
```

- [ ] **Step 2: Commit**

```bash
git add skills/autospec-review/references/gap-taxonomy.md
git commit -m "feat(autospec-review): gap-taxonomy reference doc"
```

---

## Task 4: Write references/csv-schema.md

**Files:**
- Create: `skills/autospec-review/references/csv-schema.md`

- [ ] **Step 1: Write the schema doc**

```markdown
# CSV schema and lifecycle

Two files, identical schema:

- **Per-run snapshot:** `reports/autospec-review/<YYYY-MM-DD>-<run_id>.csv`
- **Append-only ledger:** `reports/autospec-review/gaps.csv`

`run_id` = `<UTC ISO compact>-<short_git_sha>` (e.g. `20260506T1430Z-6c2e3a4`).

## Columns (frozen for v1)

| # | Name | Type | Description |
|---|------|------|-------------|
| 1 | `gap_id` | str | `sha1(spec_path + spec_anchor + gap_type)[:10]` — primary key |
| 2 | `run_id` | str | run when row was written |
| 3 | `audit_date` | ISO date | yyyy-mm-dd |
| 4 | `repo` | str | `owner/name` |
| 5 | `spec_path` | str | repo-relative |
| 6 | `spec_topic` | str | filename slug (date-prefix stripped) |
| 7 | `gap_type` | enum | `ac_no_issue` \| `closed_missing_code` \| `closed_unchecked_ac` \| `section_no_coverage` |
| 8 | `severity` | enum | `blocker` \| `major` \| `minor` |
| 9 | `title` | str | short imperative |
| 10 | `spec_anchor` | str | section heading (verbatim) |
| 11 | `evidence` | str | ≤ 500 chars, newlines escaped to `\n` |
| 12 | `suspected_issues` | str | space-separated `#nnn` list |
| 13 | `remediation_issue` | str | `#nnn` once filed; empty initially |
| 14 | `remediation_pr` | str | `#nnn` once /autospec-run merges |
| 15 | `status` | enum | `open` \| `filed` \| `fixed` \| `wontfix` \| `false_positive` |
| 16 | `notes` | str | free text; user-editable; preserved across runs |

## Lifecycle

1. New gap → `status=open`.
2. After `/autospec-split` returns issue numbers → write `remediation_issue`,
   flip to `status=filed`.
3. On future runs, key by `gap_id`. If existing `status=filed` row's evidence
   no longer reproduces (artifact present, AC checked, etc.), flip to
   `status=fixed` BEFORE generating regression spec — so we don't refile.
4. Manual `wontfix` / `false_positive` are preserved across runs.
   Excluded from regression-spec generation.
5. A regression issue closed-without-merge on a later audit → flip back
   to `status=open` with note `Re-opened on <run_id>: closed issue
   #<nnn> was not merged`.

## CSV escaping rules

- `evidence` column: replace literal `\n` with `\\n` AND `\r` with `\\r`
  AND `"` with `""` (CSV double-quote convention). Truncate at 500 chars
  AFTER escaping; append `…` if truncated.
- All other str columns: standard CSV double-quote convention.
- Always emit headers row.
- File is UTF-8, LF line endings.

## Append vs replace logic

- Per-run snapshot: replace if same `run_id` already exists; otherwise
  fresh write.
- Ledger: read existing, merge by `gap_id`, write atomically (write to
  `.tmp` then `os.rename`). Existing manual annotations
  (`status` ∈ {wontfix, false_positive} OR `notes` ≠ "") are preserved
  — script must not overwrite these columns.
```

- [ ] **Step 2: Commit**

```bash
git add skills/autospec-review/references/csv-schema.md
git commit -m "feat(autospec-review): csv-schema reference doc"
```

---

## Task 5: Write references/subagent-contract.md

**Files:**
- Create: `skills/autospec-review/references/subagent-contract.md`

- [ ] **Step 1: Write the contract doc**

```markdown
# Audit subagent contract

The skill dispatches one Tier-A subagent per spec. Each subagent
receives a YAML input block and must return a JSON object matching the
schema below. The skill validates the JSON; on schema failure it
re-runs the subagent ONCE with the validation error appended to the
prompt, then gives up and writes a `notes=subagent schema failure` row
with severity `blocker` for the whole spec.

## Subagent input (YAML)

```
spec_path: docs/specs/2026-04-30-source-completeness-remediation-design.md
spec_text: |
  <full body of the spec, ≤ 80k tokens>
linked_issues:
  - num: 472
    state: closed
    title: "..."
    body: "..."
    labels: ["auto-implement", "completeness"]
    pr: 488
    pr_state: merged
  - ...
repo_root: /abs/path/to/target/repo
audit_date: 2026-05-06
run_id: 20260506T1430Z-6c2e3a4
gap_taxonomy: |
  <full contents of references/gap-taxonomy.md>
```

## Subagent output (JSON)

```json
{
  "spec_path": "docs/specs/...",
  "no_gaps_confidence": 0.0,
  "gaps": [
    {
      "gap_type": "closed_missing_code",
      "severity": "blocker",
      "title": "NLM source schema migration not present",
      "spec_anchor": "## 4.2 NLM source schema",
      "evidence": "Issue #488 (merged in PR #491) cites src/.../schema.py — file is absent.",
      "suspected_issues": ["#472", "#488"],
      "remediation_hint": "Land src/.../schema.py with columns enumerated in spec §4.2; add tests/.../test_schema.py covering validation."
    }
  ]
}
```

## Schema rules

- Top-level keys: `spec_path` (str, must equal input's spec_path),
  `gaps` (array, may be empty), `no_gaps_confidence` (float 0.0–1.0,
  REQUIRED only when `gaps == []`; ignored otherwise).
- Each gap MUST have all 7 fields. No extra fields permitted (forward
  compat: skill may add fields later, but subagent must not invent
  them).
- `gap_type` ∈ taxonomy enum.
- `severity` ∈ `blocker` | `major` | `minor`.
- `evidence` ≤ 500 chars (skill truncates if over).
- `spec_anchor` MUST appear verbatim in `spec_text` (skill verifies).
- `suspected_issues` items MUST exist in `linked_issues` (skill
  verifies).

## Tier and tools

- **Tier:** A (top model + ultrathink) — see AGENTS.md.
- **Tools available to subagent:** Read, Bash (rg, find, test, pytest
  --collect-only), Grep, WebFetch (for cross-referencing only),
  general-purpose Agent (rare; only for follow-up sub-explorations).
- **Tools NOT available:** Write, Edit. Subagent is read-only.
```

- [ ] **Step 2: Commit**

```bash
git add skills/autospec-review/references/subagent-contract.md
git commit -m "feat(autospec-review): subagent-contract reference doc"
```

---

## Task 6: Write references/reviewer-prompt.md (§7a reviewer)

**Files:**
- Create: `skills/autospec-review/references/reviewer-prompt.md`

- [ ] **Step 1: Write the prompt doc**

```markdown
# Reviewer subagent prompt (§7a)

Dispatched once per regression spec, BEFORE the spec is committed and
BEFORE `/autospec-split` runs. Tier A.

## Input (yaml block in subagent prompt)

```
parent_spec_path: docs/specs/<original-spec>.md
parent_spec_text: |
  <full body of original spec>
regression_spec_path: docs/specs/<date>-<orig-slug>-regressions.md
regression_spec_text: |
  <full body of rendered regression spec>
gap_rows:
  - gap_id: a3f291b8c4
    gap_type: closed_missing_code
    severity: blocker
    spec_anchor: "## 4.2 NLM source schema"
    title: "NLM source schema migration not present"
    evidence: "..."
    suspected_issues: ["#472", "#488"]
    remediation_hint: "..."
  - ...
```

## Reviewer task

Review this regression spec and the parent spec it points to.

For each gap_id, ask: **does the gap description actually match what the
parent spec required?**

Flag any gap that looks like:
- A false positive (artifact exists at a renamed path; AC was
  semantically covered by another issue; section is purely background)
- An over-broad ask (gap demands more than the parent spec required)
- A YAGNI violation (gap demands behaviour the parent spec listed as
  Non-goal or Future)
- An ambiguous AC bullet that invites scope creep when implemented

Suggest tightening of AC bullets that lack concrete verification.

## Output (JSON)

```json
{
  "regression_spec_path": "docs/specs/...",
  "false_positive_gap_ids": ["a3f291b8c4"],
  "scope_concern_gap_ids": ["7d1e0c2af9"],
  "ac_tightening": [
    {
      "gap_id": "...",
      "original_ac": "- [ ] Land src/.../schema.py",
      "suggested_ac": "- [ ] Land src/.../schema.py with at minimum columns: source_id, source_name, last_synced_at"
    }
  ],
  "reviewer_notes_md": "Markdown text to be appended to regression spec under '### Reviewer notes'. Include reasoning for each flag."
}
```

## Skill behaviour after reviewer returns

1. CSV rows in `false_positive_gap_ids` flip to `status=false_positive`
   with note `Reviewer flagged: <reviewer reason>`.
2. Those gaps are stripped from the regression spec body (entire
   `### Gap <id>` section removed).
3. Gaps in `scope_concern_gap_ids` keep `status=open` but get a note
   prepended `Reviewer scope-concern: <reason>`.
4. `ac_tightening` suggestions REPLACE the corresponding AC bullet in
   the regression spec.
5. `reviewer_notes_md` is appended to regression spec under a new
   `### Reviewer notes (autospec-review §7a, Tier A, <run_id>)` heading.

## Tier and tools

- **Tier:** A (top model + ultrathink). Same class as Phase 3.5 review.
- **Tools available:** Read, Grep, Bash (read-only commands).
- **Tools NOT available:** Write, Edit (skill applies the JSON output to
  the file; reviewer never edits directly).
```

- [ ] **Step 2: Commit**

```bash
git add skills/autospec-review/references/reviewer-prompt.md
git commit -m "feat(autospec-review): reviewer-prompt reference doc"
```

---

## Task 7: Write templates/regression-spec.md.tmpl

**Files:**
- Create: `skills/autospec-review/templates/regression-spec.md.tmpl`

- [ ] **Step 1: Write the template**

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

{{parent_spec_summary}}

## Gaps to remediate

{{#each gaps}}
### Gap {{gap_id}} — `{{gap_type}}` — {{severity}}

**Parent spec anchor:** {{spec_anchor}}
**Suspected closed issues:** {{suspected_issues}}
**Evidence:**
> {{evidence}}

**What needs to ship:** {{remediation_hint}}

**Acceptance criteria:**
{{ac_bullets}}
- [ ] Verification: {{verification_target}}

{{/each}}

## Verification

- [ ] Test suite for affected modules passes.
- [ ] Each gap_id's evidence no longer reproduces (verified via re-grep).
- [ ] CSV rows flip from `status=filed` to `status=fixed` on next audit.
```

- [ ] **Step 2: Commit**

```bash
git add skills/autospec-review/templates/regression-spec.md.tmpl
git commit -m "feat(autospec-review): regression-spec template"
```

---

## Task 8: TDD — `gap_id` deterministic hash (script foundation)

**Files:**
- Create: `tests/test_autospec_review.py`
- Create: `scripts/autospec_review_audit.py`

- [ ] **Step 1: Write failing test for `gap_id` computation**

```python
# tests/test_autospec_review.py
"""Unit tests for scripts/autospec_review_audit.py."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

import autospec_review_audit as ara


def test_gap_id_is_deterministic():
    a = ara.compute_gap_id(
        spec_path="docs/specs/2026-04-30-foo.md",
        spec_anchor="## 4.2 NLM source schema",
        gap_type="closed_missing_code",
    )
    b = ara.compute_gap_id(
        spec_path="docs/specs/2026-04-30-foo.md",
        spec_anchor="## 4.2 NLM source schema",
        gap_type="closed_missing_code",
    )
    assert a == b
    assert len(a) == 10
    assert all(c in "0123456789abcdef" for c in a)


def test_gap_id_changes_on_input_change():
    base = ara.compute_gap_id("a.md", "## H", "ac_no_issue")
    assert ara.compute_gap_id("b.md", "## H", "ac_no_issue") != base
    assert ara.compute_gap_id("a.md", "## I", "ac_no_issue") != base
    assert ara.compute_gap_id("a.md", "## H", "section_no_coverage") != base
```

- [ ] **Step 2: Run test — expect ImportError or AttributeError**

```bash
pytest tests/test_autospec_review.py::test_gap_id_is_deterministic -v
```
Expected: FAIL with `ModuleNotFoundError` or `AttributeError`.

- [ ] **Step 3: Implement minimal `compute_gap_id`**

```python
# scripts/autospec_review_audit.py
"""Deterministic helpers for the autospec-review skill.

Public functions are called both from skill body (via shell) and from
unit tests.  No LLM calls; no network calls except `gh`.
"""
from __future__ import annotations

import hashlib


def compute_gap_id(spec_path: str, spec_anchor: str, gap_type: str) -> str:
    """Stable 10-hex-char primary key for a gap.

    Renaming a section header creates a new ``gap_id`` (treated as a new
    gap).  Stability across audit runs is what protects manual ledger
    annotations.
    """
    payload = f"{spec_path}\n{spec_anchor}\n{gap_type}".encode("utf-8")
    return hashlib.sha1(payload).hexdigest()[:10]
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
pytest tests/test_autospec_review.py -v
```
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add tests/test_autospec_review.py scripts/autospec_review_audit.py
git commit -m "feat(autospec-review): compute_gap_id deterministic primary key"
```

---

## Task 9: TDD — spec discovery

**Files:**
- Modify: `tests/test_autospec_review.py` (append)
- Modify: `scripts/autospec_review_audit.py` (append)

- [ ] **Step 1: Write failing tests**

```python
# append to tests/test_autospec_review.py

def test_discover_specs_default_globs(tmp_path):
    (tmp_path / "docs/specs").mkdir(parents=True)
    (tmp_path / "docs/superpowers/specs").mkdir(parents=True)
    a = tmp_path / "docs/specs/2026-04-30-alpha-design.md"
    b = tmp_path / "docs/superpowers/specs/2026-04-23-beta-design.md"
    c = tmp_path / "docs/specs/notes.txt"   # NOT a spec
    a.write_text("# Alpha\n")
    b.write_text("# Beta\n")
    c.write_text("just notes\n")

    found = ara.discover_specs(repo_root=tmp_path)
    paths = sorted(p.spec_path for p in found)
    assert paths == [
        "docs/specs/2026-04-30-alpha-design.md",
        "docs/superpowers/specs/2026-04-23-beta-design.md",
    ]


def test_discover_specs_extracts_topic_and_date(tmp_path):
    (tmp_path / "docs/specs").mkdir(parents=True)
    (tmp_path / "docs/specs/2026-04-30-alpha-beta-design.md").write_text("# x")
    (tmp_path / "docs/specs/no-date-design.md").write_text("# y")

    found = {p.spec_path: p for p in ara.discover_specs(repo_root=tmp_path)}
    p1 = found["docs/specs/2026-04-30-alpha-beta-design.md"]
    assert p1.spec_topic == "alpha-beta"
    assert p1.spec_date == "2026-04-30"

    p2 = found["docs/specs/no-date-design.md"]
    assert p2.spec_topic == "no-date"
    assert p2.spec_date is None


def test_discover_specs_honors_glob_override(tmp_path):
    (tmp_path / "weird/place").mkdir(parents=True)
    (tmp_path / "weird/place/spec.md").write_text("# z")

    found = ara.discover_specs(
        repo_root=tmp_path, globs=("weird/**/*.md",)
    )
    assert [p.spec_path for p in found] == ["weird/place/spec.md"]
```

- [ ] **Step 2: Run tests — expect FAIL (`AttributeError`)**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 3: Implement**

```python
# append to scripts/autospec_review_audit.py
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence

DEFAULT_SPEC_GLOBS: tuple[str, ...] = (
    "docs/specs/**/*.md",
    "docs/superpowers/specs/**/*.md",
)

_DATE_PREFIX = re.compile(r"^(?P<date>\d{4}-\d{2}-\d{2})-(?P<rest>.+?)(?:-design)?$")


@dataclass(frozen=True)
class SpecRef:
    spec_path: str             # repo-relative path
    spec_topic: str            # filename slug, date-prefix and -design suffix stripped
    spec_date: str | None      # yyyy-mm-dd or None
    abs_path: Path             # absolute path, useful for callers


def discover_specs(
    repo_root: Path,
    globs: Sequence[str] = DEFAULT_SPEC_GLOBS,
) -> list[SpecRef]:
    repo_root = Path(repo_root)
    seen: dict[str, SpecRef] = {}
    for pattern in globs:
        for abs_path in sorted(repo_root.glob(pattern)):
            if not abs_path.is_file():
                continue
            rel = abs_path.relative_to(repo_root).as_posix()
            stem = abs_path.stem
            m = _DATE_PREFIX.match(stem)
            if m:
                topic = m.group("rest")
                date = m.group("date")
            else:
                topic = stem.removesuffix("-design")
                date = None
            seen[rel] = SpecRef(
                spec_path=rel, spec_topic=topic, spec_date=date, abs_path=abs_path
            )
    return list(seen.values())
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 5: Commit**

```bash
git add tests/test_autospec_review.py scripts/autospec_review_audit.py
git commit -m "feat(autospec-review): spec discovery (date prefix, glob override)"
```

---

## Task 10: TDD — issue linkage matrix

**Files:**
- Modify: `tests/test_autospec_review.py` (append)
- Modify: `scripts/autospec_review_audit.py` (append)

- [ ] **Step 1: Write failing test using fake `gh` outputs**

```python
# append to tests/test_autospec_review.py

def test_link_issues_by_inline_number():
    spec_text = "Tracker #260, fix #472, see also (#488)."
    issues = [
        {"number": 260, "state": "open",   "title": "x", "body": "", "labels": []},
        {"number": 472, "state": "closed", "title": "y", "body": "", "labels": []},
        {"number": 488, "state": "closed", "title": "z", "body": "", "labels": []},
        {"number": 999, "state": "open",   "title": "irrelevant", "body": "", "labels": []},
    ]
    linked = ara.link_issues(spec_text=spec_text, spec_path="docs/specs/foo.md",
                             spec_topic="foo", all_issues=issues)
    nums = sorted(i["number"] for i in linked)
    assert nums == [260, 472, 488]


def test_link_issues_by_spec_path_in_body():
    spec_text = ""
    issues = [
        {"number": 1, "state": "open", "title": "a", "body": "", "labels": []},
        {"number": 2, "state": "open", "title": "b",
         "body": "implements docs/specs/foo.md§3", "labels": []},
    ]
    linked = ara.link_issues(spec_text=spec_text, spec_path="docs/specs/foo.md",
                             spec_topic="foo", all_issues=issues)
    assert [i["number"] for i in linked] == [2]


def test_link_issues_by_topic_label_or_title():
    spec_text = ""
    issues = [
        {"number": 1, "state": "open", "title": "Add foo bar",      "body": "", "labels": []},
        {"number": 2, "state": "open", "title": "Unrelated",        "body": "",
         "labels": [{"name": "foo"}]},
        {"number": 3, "state": "open", "title": "totally other",    "body": "", "labels": []},
    ]
    linked = ara.link_issues(spec_text=spec_text, spec_path="docs/specs/x.md",
                             spec_topic="foo", all_issues=issues)
    assert sorted(i["number"] for i in linked) == [1, 2]
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 3: Implement**

```python
# append to scripts/autospec_review_audit.py
_INLINE_ISSUE = re.compile(r"#(\d+)")


def link_issues(
    spec_text: str,
    spec_path: str,
    spec_topic: str,
    all_issues: Iterable[dict],
) -> list[dict]:
    """Return issues linked to a spec by any of three signals.

    1. Inline ``#nnn`` references in spec_text.
    2. Spec path or its filename appearing in issue body.
    3. Topic slug appearing in issue title (case-insensitive substring)
       or in any of the issue's labels.

    Order: stable, deduplicated, sorted by issue number.
    """
    inline_nums = {int(m) for m in _INLINE_ISSUE.findall(spec_text)}
    spec_filename = spec_path.rsplit("/", 1)[-1]
    topic_lc = spec_topic.lower()

    matched: dict[int, dict] = {}
    for issue in all_issues:
        num = issue["number"]
        if num in inline_nums:
            matched[num] = issue
            continue
        body = issue.get("body") or ""
        if spec_path in body or spec_filename in body:
            matched[num] = issue
            continue
        if topic_lc and topic_lc in (issue.get("title") or "").lower():
            matched[num] = issue
            continue
        labels = issue.get("labels") or []
        label_names = {
            (lbl["name"] if isinstance(lbl, dict) else lbl).lower()
            for lbl in labels
        }
        if topic_lc in label_names:
            matched[num] = issue
            continue
    return [matched[n] for n in sorted(matched)]
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 5: Commit**

```bash
git add tests/test_autospec_review.py scripts/autospec_review_audit.py
git commit -m "feat(autospec-review): three-signal issue linkage"
```

---

## Task 11: TDD — JSON validation for subagent output

**Files:**
- Modify: `tests/test_autospec_review.py` (append)
- Modify: `scripts/autospec_review_audit.py` (append)

- [ ] **Step 1: Write failing tests**

```python
# append to tests/test_autospec_review.py
import pytest


VALID_GAP = {
    "gap_type": "closed_missing_code",
    "severity": "blocker",
    "title": "x",
    "spec_anchor": "## H",
    "evidence": "...",
    "suspected_issues": ["#472"],
    "remediation_hint": "ship it",
}


def test_validate_subagent_output_accepts_minimal():
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [VALID_GAP]}
    ara.validate_subagent_output(
        payload,
        expected_spec_path="docs/specs/foo.md",
        spec_text="## H\n",
        linked_numbers={472},
    )


def test_validate_subagent_output_rejects_unknown_severity():
    bad = {**VALID_GAP, "severity": "wat"}
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [bad]}
    with pytest.raises(ara.SubagentSchemaError, match="severity"):
        ara.validate_subagent_output(
            payload, expected_spec_path="docs/specs/foo.md",
            spec_text="## H\n", linked_numbers={472},
        )


def test_validate_subagent_output_rejects_unknown_anchor():
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [VALID_GAP]}
    with pytest.raises(ara.SubagentSchemaError, match="spec_anchor"):
        ara.validate_subagent_output(
            payload, expected_spec_path="docs/specs/foo.md",
            spec_text="## DIFFERENT\n", linked_numbers={472},
        )


def test_validate_subagent_output_rejects_unknown_issue():
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [VALID_GAP]}
    with pytest.raises(ara.SubagentSchemaError, match="suspected_issues"):
        ara.validate_subagent_output(
            payload, expected_spec_path="docs/specs/foo.md",
            spec_text="## H\n", linked_numbers={1},
        )


def test_validate_subagent_output_truncates_evidence():
    long = "x" * 600
    gap = {**VALID_GAP, "evidence": long}
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [gap]}
    cleaned = ara.validate_subagent_output(
        payload, expected_spec_path="docs/specs/foo.md",
        spec_text="## H\n", linked_numbers={472},
    )
    assert len(cleaned["gaps"][0]["evidence"]) <= 500
    assert cleaned["gaps"][0]["evidence"].endswith("…")
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 3: Implement**

```python
# append to scripts/autospec_review_audit.py

GAP_TYPES = frozenset({
    "ac_no_issue", "closed_missing_code",
    "closed_unchecked_ac", "section_no_coverage",
})
SEVERITIES = frozenset({"blocker", "major", "minor"})
GAP_REQUIRED_FIELDS = (
    "gap_type", "severity", "title", "spec_anchor",
    "evidence", "suspected_issues", "remediation_hint",
)
EVIDENCE_MAX_CHARS = 500


class SubagentSchemaError(ValueError):
    """Raised when a subagent's JSON output fails the contract."""


def validate_subagent_output(
    payload: dict,
    *,
    expected_spec_path: str,
    spec_text: str,
    linked_numbers: set[int],
) -> dict:
    """Validate + normalise; return cleaned payload (truncated evidence).

    Raises ``SubagentSchemaError`` on contract violations.
    """
    if not isinstance(payload, dict):
        raise SubagentSchemaError("payload must be an object")

    if payload.get("spec_path") != expected_spec_path:
        raise SubagentSchemaError(
            f"spec_path mismatch: expected {expected_spec_path!r}, "
            f"got {payload.get('spec_path')!r}"
        )

    gaps = payload.get("gaps")
    if not isinstance(gaps, list):
        raise SubagentSchemaError("gaps must be a list")

    cleaned_gaps: list[dict] = []
    for idx, gap in enumerate(gaps):
        if not isinstance(gap, dict):
            raise SubagentSchemaError(f"gap[{idx}] must be object")
        for field in GAP_REQUIRED_FIELDS:
            if field not in gap:
                raise SubagentSchemaError(
                    f"gap[{idx}] missing field {field!r}"
                )
        if gap["gap_type"] not in GAP_TYPES:
            raise SubagentSchemaError(
                f"gap[{idx}] gap_type {gap['gap_type']!r} not in taxonomy"
            )
        if gap["severity"] not in SEVERITIES:
            raise SubagentSchemaError(
                f"gap[{idx}] severity {gap['severity']!r} unknown"
            )
        if gap["spec_anchor"] not in spec_text:
            raise SubagentSchemaError(
                f"gap[{idx}] spec_anchor {gap['spec_anchor']!r} "
                "not found in spec_text"
            )
        if not isinstance(gap["suspected_issues"], list):
            raise SubagentSchemaError(
                f"gap[{idx}] suspected_issues must be a list"
            )
        for ref in gap["suspected_issues"]:
            try:
                num = int(str(ref).lstrip("#"))
            except ValueError as e:
                raise SubagentSchemaError(
                    f"gap[{idx}] suspected_issues item {ref!r} not parseable"
                ) from e
            if num not in linked_numbers:
                raise SubagentSchemaError(
                    f"gap[{idx}] suspected_issues #{num} not in linked set"
                )

        cleaned_gap = dict(gap)
        ev = cleaned_gap.get("evidence", "")
        if len(ev) > EVIDENCE_MAX_CHARS:
            cleaned_gap["evidence"] = ev[: EVIDENCE_MAX_CHARS - 1] + "…"
        cleaned_gaps.append(cleaned_gap)

    cleaned = dict(payload)
    cleaned["gaps"] = cleaned_gaps
    return cleaned
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 5: Commit**

```bash
git add tests/test_autospec_review.py scripts/autospec_review_audit.py
git commit -m "feat(autospec-review): JSON contract validation with truncation"
```

---

## Task 12: TDD — CSV writer with manual-edit preservation

**Files:**
- Modify: `tests/test_autospec_review.py` (append)
- Modify: `scripts/autospec_review_audit.py` (append)

- [ ] **Step 1: Write failing tests**

```python
# append to tests/test_autospec_review.py
import csv


def _row(**overrides):
    base = {
        "gap_id": "abc123def0",
        "run_id": "20260506T1430Z-deadbee",
        "audit_date": "2026-05-06",
        "repo": "owner/repo",
        "spec_path": "docs/specs/foo.md",
        "spec_topic": "foo",
        "gap_type": "closed_missing_code",
        "severity": "blocker",
        "title": "missing thing",
        "spec_anchor": "## H",
        "evidence": "ev",
        "suspected_issues": "#1 #2",
        "remediation_issue": "",
        "remediation_pr": "",
        "status": "open",
        "notes": "",
    }
    base.update(overrides)
    return base


def test_write_per_run_csv_round_trip(tmp_path):
    rows = [_row(), _row(gap_id="0000000000", title="other")]
    out = tmp_path / "snapshot.csv"
    ara.write_per_run_csv(out, rows)
    with out.open() as f:
        loaded = list(csv.DictReader(f))
    assert len(loaded) == 2
    assert loaded[0]["gap_id"] == "abc123def0"
    assert list(loaded[0].keys()) == list(ara.CSV_COLUMNS)


def test_merge_into_ledger_preserves_manual_status(tmp_path):
    ledger = tmp_path / "gaps.csv"
    existing = _row(gap_id="keep1", status="wontfix",
                    notes="manual: not applicable for v1")
    ara.write_per_run_csv(ledger, [existing])

    new = _row(gap_id="keep1", status="open", notes="")
    ara.merge_into_ledger(ledger, [new, _row(gap_id="newrow")])

    with ledger.open() as f:
        merged = {r["gap_id"]: r for r in csv.DictReader(f)}
    assert merged["keep1"]["status"] == "wontfix"
    assert merged["keep1"]["notes"] == "manual: not applicable for v1"
    assert merged["newrow"]["status"] == "open"


def test_merge_into_ledger_atomic_via_tmp(tmp_path):
    ledger = tmp_path / "gaps.csv"
    ara.write_per_run_csv(ledger, [_row()])
    ara.merge_into_ledger(ledger, [_row(gap_id="another")])
    assert not (tmp_path / "gaps.csv.tmp").exists()
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 3: Implement**

```python
# append to scripts/autospec_review_audit.py
import csv
import os

CSV_COLUMNS: tuple[str, ...] = (
    "gap_id", "run_id", "audit_date", "repo",
    "spec_path", "spec_topic", "gap_type", "severity",
    "title", "spec_anchor", "evidence", "suspected_issues",
    "remediation_issue", "remediation_pr", "status", "notes",
)
PRESERVED_STATUSES = frozenset({"wontfix", "false_positive"})


def write_per_run_csv(out_path: Path, rows: Iterable[dict]) -> None:
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=CSV_COLUMNS, lineterminator="\n")
        writer.writeheader()
        for row in rows:
            writer.writerow({k: row.get(k, "") for k in CSV_COLUMNS})


def merge_into_ledger(ledger_path: Path, new_rows: Iterable[dict]) -> None:
    """Merge new_rows into ledger keyed by gap_id, preserving manual edits.

    Manual edits = rows whose existing ``status`` is in ``PRESERVED_STATUSES``,
    OR whose ``notes`` field is non-empty.  Those rows' ``status`` and
    ``notes`` columns are NOT overwritten by new_rows.  All other columns
    are refreshed from new_rows.
    """
    ledger_path = Path(ledger_path)
    existing: dict[str, dict] = {}
    if ledger_path.exists():
        with ledger_path.open(encoding="utf-8") as f:
            for row in csv.DictReader(f):
                existing[row["gap_id"]] = row

    for new in new_rows:
        gid = new["gap_id"]
        prior = existing.get(gid)
        if prior is None:
            existing[gid] = {k: new.get(k, "") for k in CSV_COLUMNS}
            continue
        merged = {k: new.get(k, "") for k in CSV_COLUMNS}
        if prior.get("status") in PRESERVED_STATUSES or prior.get("notes"):
            merged["status"] = prior.get("status", merged["status"])
            merged["notes"] = prior.get("notes", merged["notes"])
        existing[gid] = merged

    tmp_path = ledger_path.with_suffix(ledger_path.suffix + ".tmp")
    with tmp_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=CSV_COLUMNS, lineterminator="\n")
        writer.writeheader()
        for row in existing.values():
            writer.writerow({k: row.get(k, "") for k in CSV_COLUMNS})
    os.replace(tmp_path, ledger_path)
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 5: Commit**

```bash
git add tests/test_autospec_review.py scripts/autospec_review_audit.py
git commit -m "feat(autospec-review): CSV writer with manual-edit preservation"
```

---

## Task 13: TDD — `run_id` generation + `gh issue list` adapter

**Files:**
- Modify: `tests/test_autospec_review.py` (append)
- Modify: `scripts/autospec_review_audit.py` (append)

- [ ] **Step 1: Write failing tests**

```python
# append to tests/test_autospec_review.py
import re


def test_run_id_format():
    rid = ara.generate_run_id(short_sha="6c2e3a4")
    assert re.match(r"^\d{8}T\d{4}Z-6c2e3a4$", rid)


def test_run_id_includes_provided_sha():
    assert ara.generate_run_id(short_sha="abc1234").endswith("-abc1234")
```

- [ ] **Step 2: Run tests — expect FAIL**

- [ ] **Step 3: Implement**

```python
# append to scripts/autospec_review_audit.py
import datetime as _dt
import shutil
import subprocess


def generate_run_id(short_sha: str | None = None) -> str:
    """``<UTC compact ISO>-<short_git_sha>`` — sortable + traceable."""
    if short_sha is None:
        short_sha = current_git_short_sha()
    ts = _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%MZ")
    return f"{ts}-{short_sha}"


def current_git_short_sha() -> str:
    return subprocess.check_output(
        ["git", "rev-parse", "--short", "HEAD"], text=True
    ).strip()


def gh_issue_list(repo: str, *, state: str = "all", limit: int = 1000) -> list[dict]:
    """Wrapper around ``gh issue list --json ...``.

    Returns the parsed JSON list.  Raises ``RuntimeError`` if ``gh`` is
    not on PATH.
    """
    if shutil.which("gh") is None:
        raise RuntimeError("gh CLI not on PATH; install GitHub CLI")
    out = subprocess.check_output([
        "gh", "issue", "list",
        "--repo", repo,
        "--state", state,
        "--limit", str(limit),
        "--json", "number,state,title,body,labels,closedAt,url",
    ], text=True)
    import json
    return json.loads(out)
```

- [ ] **Step 4: Run tests — expect PASS**

- [ ] **Step 5: Commit**

```bash
git add tests/test_autospec_review.py scripts/autospec_review_audit.py
git commit -m "feat(autospec-review): run_id generator and gh issue list adapter"
```

---

## Task 13b: TDD — argparse CLI entry point

**Files:**
- Modify: `tests/test_autospec_review.py` (append)
- Modify: `scripts/autospec_review_audit.py` (append)

The skill body (Tasks 14–15) invokes the script as a CLI with subcommands
`discover`, `link`, `validate-subagent`, `write-csv`, `update-status`.
This task wires those up.

- [ ] **Step 1: Write failing tests using `subprocess`**

```python
# append to tests/test_autospec_review.py
import json
import subprocess


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "autospec_review_audit.py"


def test_cli_discover_writes_json(tmp_path):
    (tmp_path / "docs/specs").mkdir(parents=True)
    (tmp_path / "docs/specs/2026-04-30-foo-design.md").write_text("# foo\n")
    out = tmp_path / "specs.json"
    subprocess.check_call([
        "python", str(SCRIPT), "discover",
        "--repo-root", str(tmp_path),
        "--out", str(out),
    ])
    data = json.loads(out.read_text())
    assert len(data) == 1
    assert data[0]["spec_topic"] == "foo"


def test_cli_unknown_subcommand_errors():
    res = subprocess.run(
        ["python", str(SCRIPT), "wat"],
        capture_output=True, text=True,
    )
    assert res.returncode != 0
    assert "wat" in (res.stderr + res.stdout) or "invalid choice" in res.stderr


def test_cli_write_csv_emits_snapshot_and_ledger(tmp_path):
    rows_file = tmp_path / "rows.json"
    rows_file.write_text(json.dumps([_row()]))
    snapshot = tmp_path / "snap.csv"
    ledger = tmp_path / "ledger.csv"
    subprocess.check_call([
        "python", str(SCRIPT), "write-csv",
        "--rows", str(rows_file),
        "--snapshot", str(snapshot),
        "--ledger", str(ledger),
    ])
    assert snapshot.exists() and ledger.exists()
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 3: Implement argparse dispatch**

Append to `scripts/autospec_review_audit.py`:

```python
# append to scripts/autospec_review_audit.py
import argparse
import json as _json
import sys


def _cli_discover(args: argparse.Namespace) -> int:
    repo_root = Path(args.repo_root)
    globs = (args.glob,) if args.glob else DEFAULT_SPEC_GLOBS
    specs = discover_specs(repo_root, globs=globs)
    if args.since:
        specs = [s for s in specs if (s.spec_date or "9999") >= args.since]
    payload = [
        {"spec_path": s.spec_path, "spec_topic": s.spec_topic,
         "spec_date": s.spec_date}
        for s in specs
    ]
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(_json.dumps(payload, indent=2))
    return 0


def _cli_link(args: argparse.Namespace) -> int:
    specs_meta = _json.loads(Path(args.specs).read_text())
    issues = gh_issue_list(args.repo, state="all", limit=args.limit)
    repo_root = Path(args.repo_root or ".")
    out_payload = []
    for meta in specs_meta:
        spec_text = (repo_root / meta["spec_path"]).read_text(encoding="utf-8")
        linked = link_issues(
            spec_text=spec_text,
            spec_path=meta["spec_path"],
            spec_topic=meta["spec_topic"],
            all_issues=issues,
        )
        out_payload.append({
            "spec_path": meta["spec_path"],
            "spec_topic": meta["spec_topic"],
            "spec_text": spec_text,
            "linked_issues": linked,
        })
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(_json.dumps(out_payload, indent=2))
    return 0


def _cli_validate_subagent(args: argparse.Namespace) -> int:
    payload = _json.loads(Path(args.input).read_text())
    spec_text = Path(args.spec_text_file).read_text(encoding="utf-8")
    linked_numbers = {int(n) for n in args.linked_numbers.split()}
    cleaned = validate_subagent_output(
        payload,
        expected_spec_path=args.spec_path,
        spec_text=spec_text,
        linked_numbers=linked_numbers,
    )
    Path(args.out or args.input).write_text(_json.dumps(cleaned, indent=2))
    return 0


def _cli_write_csv(args: argparse.Namespace) -> int:
    rows = _json.loads(Path(args.rows).read_text())
    write_per_run_csv(Path(args.snapshot), rows)
    merge_into_ledger(Path(args.ledger), rows)
    return 0


def _cli_update_status(args: argparse.Namespace) -> int:
    """In-place update: set status (and optionally remediation_issue) for gap_id."""
    ledger = Path(args.ledger)
    rows = list(csv.DictReader(ledger.open(encoding="utf-8")))
    for row in rows:
        if row["gap_id"] == args.gap_id:
            row["status"] = args.status
            if args.issue:
                row["remediation_issue"] = args.issue
            if args.pr:
                row["remediation_pr"] = args.pr
    write_per_run_csv(ledger, rows)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="autospec_review_audit")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("discover")
    p.add_argument("--repo-root", default=".")
    p.add_argument("--glob")
    p.add_argument("--since")
    p.add_argument("--out", required=True)
    p.set_defaults(fn=_cli_discover)

    p = sub.add_parser("link")
    p.add_argument("--repo", required=True)
    p.add_argument("--repo-root", default=".")
    p.add_argument("--specs", required=True)
    p.add_argument("--limit", type=int, default=1000)
    p.add_argument("--out", required=True)
    p.set_defaults(fn=_cli_link)

    p = sub.add_parser("validate-subagent")
    p.add_argument("--input", required=True)
    p.add_argument("--spec-path", required=True)
    p.add_argument("--spec-text-file", required=True)
    p.add_argument("--linked-numbers", required=True,
                   help="space-separated issue numbers")
    p.add_argument("--out")
    p.set_defaults(fn=_cli_validate_subagent)

    p = sub.add_parser("write-csv")
    p.add_argument("--rows", required=True)
    p.add_argument("--snapshot", required=True)
    p.add_argument("--ledger", required=True)
    p.set_defaults(fn=_cli_write_csv)

    p = sub.add_parser("update-status")
    p.add_argument("--ledger", required=True)
    p.add_argument("--gap-id", required=True)
    p.add_argument("--status", required=True,
                   choices=("open", "filed", "fixed", "wontfix", "false_positive"))
    p.add_argument("--issue")
    p.add_argument("--pr")
    p.set_defaults(fn=_cli_update_status)

    args = parser.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
pytest tests/test_autospec_review.py -v
```

- [ ] **Step 5: Commit**

```bash
git add tests/test_autospec_review.py scripts/autospec_review_audit.py
git commit -m "feat(autospec-review): argparse CLI dispatch (discover/link/validate-subagent/write-csv/update-status)"
```

---

## Task 14: Write SKILL.md body — Phases 0–3 (preflight, discovery, dispatch, merge)

**Files:**
- Modify: `skills/autospec-review/SKILL.md`

This task fills in the body between `<!-- BODY START -->` and `<!-- BODY END -->`. The body documents the orchestrator's exact phases. **Do NOT** copy to opencode/codex variants yet — Task 16 lock-steps after the full body lands.

- [ ] **Step 1: Replace placeholders with the Phase 0–3 content**

Body content (replaces everything between BODY START/END):

````markdown
# autospec-review

Audit design specs against open + closed issues, write gap rows to a
CSV ledger, and route regressions back through `/autospec-split` with
`priority:high` + `regression` labels.

## When to invoke

- Manually as `/autospec-review [...flags]`.
- Automatically by `autospec-run` after the last issue in a batch
  closes — gated by `~/.autospec/no-review.flag` and the
  `--no-postreview` flag.

## CLI flags

| Flag | Effect |
|------|--------|
| `--spec PATH` | Audit one spec only |
| `--profile NAME` | Model profile from `~/.autospec/model-profiles.yml` |
| `--dry-run` | CSV + regression specs only; skip `/autospec-split` |
| `--no-autoreview` | Skip the §7a Tier-A reviewer pass |
| `--since DATE` | Only audit specs whose date prefix ≥ DATE |
| `--spec-glob PATTERN` | Override default spec discovery globs |

## Phase 0 — Preflight

1. Read repo state. Resolve `repo` from `gh repo view --json
   nameWithOwner` and `short_sha` from `git rev-parse --short HEAD`.
2. Compute `run_id = <UTC compact>-<short_sha>` (delegate to
   `scripts/autospec_review_audit.py`).
3. Acquire `~/.autospec/review.lock` (PID + start time). If lock exists
   and PID is alive AND start_time < 1h ago → exit with error
   "another autospec-review run in progress (PID X)". If stale → reclaim.
4. Ensure `reports/autospec-review/` exists in target repo.
5. Compute `audit_date = today (yyyy-mm-dd)`.

## Phase 1 — Spec discovery + linkage matrix (deterministic)

Invoke the helper script directly (no LLM):

```bash
python scripts/autospec_review_audit.py discover \
  --repo-root . \
  --since "${SINCE:-1900-01-01}" \
  ${SPEC_GLOB:+--glob "$SPEC_GLOB"} \
  --out /tmp/autospec-review/specs.json
```

Then for each spec, build the linkage:

```bash
python scripts/autospec_review_audit.py link \
  --repo "$REPO" \
  --specs /tmp/autospec-review/specs.json \
  --out  /tmp/autospec-review/linkage.json
```

Output `linkage.json` is a list of:

```json
{
  "spec_path": "...", "spec_topic": "...", "spec_text": "...",
  "linked_issues": [ ...gh issue records... ]
}
```

## Phase 2 — Audit subagent fan-out (Tier A)

For each entry in `linkage.json`, dispatch one Tier-A subagent in
batches of `${AUTOSPEC_REVIEW_BATCH_SIZE:-5}` (parallel within a batch,
serial across batches).

**Model tier:** Tier A (spec work) — top model + ultrathink.

**Subagent prompt skeleton** (verbatim, with input substitutions):

```
You are an autospec audit subagent. Read references/gap-taxonomy.md
and references/subagent-contract.md (loaded inline below). Apply the
taxonomy to the supplied spec + linked issues. Output JSON matching
the contract. Do not ship false positives — when uncertain, omit.

== gap-taxonomy.md ==
<verbatim contents>

== subagent-contract.md ==
<verbatim contents>

== input ==
<yaml block from §4 of the design spec>
```

For each subagent's JSON return:

1. `python scripts/autospec_review_audit.py validate-subagent
   --input /tmp/.../subagent-NN.json
   --spec-path "..." --linked-numbers "1 2 3" --spec-text-file ...`
2. On schema failure, retry the subagent ONCE with the validation error
   prepended to the prompt. On second failure, write a fallback row
   `{gap_type: ac_no_issue, severity: blocker, title: "subagent
   schema failure", notes: <error>, ...}`.

## Phase 3 — CSV merge + per-run snapshot

Aggregate all subagent outputs into one rows list. For each gap:

- compute `gap_id = sha1(spec_path + spec_anchor + gap_type)[:10]`
- set `status = open`, `remediation_issue = ""`, `remediation_pr = ""`
- copy `run_id`, `audit_date`, `repo` from preflight

Then:

```bash
python scripts/autospec_review_audit.py write-csv \
  --rows /tmp/autospec-review/rows.json \
  --snapshot reports/autospec-review/${AUDIT_DATE}-${RUN_ID}.csv \
  --ledger   reports/autospec-review/gaps.csv
```

The script writes the per-run snapshot (overwrite-on-same-run_id) and
merges into the ledger preserving manual `wontfix` / `false_positive`
edits.

(Phases 4–6 in next sections of this body.)
````

- [ ] **Step 2: Verify the write replaced the BODY block**

```bash
sed -n '/BODY START/,/BODY END/p' skills/autospec-review/SKILL.md | head -5
```
Expected: shows the start of the new body content, not the placeholder.

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-review/SKILL.md
git commit -m "feat(autospec-review): SKILL.md phases 0-3 (preflight, discovery, audit, csv)"
```

---

## Task 15: Write SKILL.md body — Phases 4–6 (regression spec, reviewer, /autospec-split, post-process)

**Files:**
- Modify: `skills/autospec-review/SKILL.md`

- [ ] **Step 1: Append phases 4–6 above the `<!-- BODY END -->` marker**

````markdown
## Phase 4 — Render regression specs

Group rows by `spec_path`. For each group with ≥1 `status=open` row:

1. Render `templates/regression-spec.md.tmpl` with substitutions
   (`audit_date`, `spec_path`, `run_id`, `spec_topic`,
   `parent_spec_summary` (2-3 line auto-generated summary from
   spec_text headings), and the iterated `gaps`).
2. Write to `docs/specs/${AUDIT_DATE}-${SPEC_TOPIC}-regressions.md`
   in the TARGET repo (NOT the autospec repo).
3. Do NOT commit yet — Phase 5 reviews and may modify.

## Phase 5 — Reviewer subagent (§7a Tier A)

Skip if `--no-autoreview` was passed.

For each rendered regression spec, dispatch one Tier-A reviewer
subagent.

**Model tier:** Tier A (spec work).

**Prompt:** load `references/reviewer-prompt.md` verbatim and append
the input yaml block.

For each reviewer JSON return:

1. For each `gap_id` in `false_positive_gap_ids`:
   - Update CSV row: `status=false_positive`, prepend
     `Reviewer flagged: <reason>` to `notes`.
   - Strip the corresponding `### Gap <id>` section from the
     regression spec.
2. For each `gap_id` in `scope_concern_gap_ids`:
   - Prepend `Reviewer scope-concern: <reason>` to `notes`. Keep
     `status=open`.
3. For each `ac_tightening` entry: replace the AC bullet in the
   regression spec body.
4. Append `reviewer_notes_md` under a new heading
   `### Reviewer notes (autospec-review §7a, Tier A, <run_id>)`.

After all reviewers finish:

```bash
git checkout -b "autospec-review/${RUN_ID}"
git add docs/specs/${AUDIT_DATE}-*-regressions.md
git commit -m "docs(autospec-review): regression specs from run ${RUN_ID}"
```

## Phase 6 — /autospec-split handoff + post-process

Skip if `--dry-run` was passed.

For each regression spec file:

1. Invoke `/autospec-split docs/specs/<file>` and capture the issue
   numbers it returns (parse from gh output).
2. For each new issue number:
   - `gh issue edit <num> --add-label priority:high --add-label
     regression --add-label <topic-label>`
   - `gh issue edit <num> --title "[REGRESSION] $(gh issue view <num>
     --json title -q .title)"` (idempotent — strip duplicate prefix
     first via shell prefix-test)
   - `gh issue comment <num> --body "Generated by autospec-review run
     ${RUN_ID}. See gap_id <id> in
     reports/autospec-review/gaps.csv."`
3. Update CSV rows: write `remediation_issue=#<num>`, flip
   `status=filed`. Use `python scripts/autospec_review_audit.py
   update-status --gap-id <id> --status filed --issue <num>`.

## Finalization

1. Append run summary to `reports/autospec-review/runs.md` (newest
   first).
2. Print to console: `run_id`, gaps by type, gaps by severity,
   regression issues filed, paths to per-run CSV + ledger.
3. If env `AUTOSPEC_REVIEW_NOTIFY` set, POST the same summary as JSON
   to that webhook.
4. Release `~/.autospec/review.lock`.
````

- [ ] **Step 2: Verify body sections are in order**

```bash
grep -n "^## Phase" skills/autospec-review/SKILL.md
```
Expected: Phases 0, 1, 2, 3, 4, 5, 6 in order.

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-review/SKILL.md
git commit -m "feat(autospec-review): SKILL.md phases 4-6 (render, review, split, post)"
```

---

## Task 16: Lock-step copy to opencode + codex variants

**Files:**
- Modify: `skills/autospec-review/opencode/agent.md`
- Modify: `skills/autospec-review/codex/prompt.md`

The autospec convention: SKILL.md body (everything after the second
`---`) must be byte-identical across the trio. Frontmatters differ.

- [ ] **Step 1: Extract canonical body**

```bash
awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md > /tmp/autospec-review-body.md
```

- [ ] **Step 2: Build opencode/agent.md (description + mode frontmatter, then body)**

```bash
cat > skills/autospec-review/opencode/agent.md <<'HEADER'
---
description: Audit design specs against open + closed issues to find gaps, file high-priority regression issues, and feed them back through autospec. Runs as `/autospec-review` (manual) or auto-fires after each autospec-run batch unless `~/.autospec/no-review.flag` exists.
mode: primary
---

HEADER
cat /tmp/autospec-review-body.md >> skills/autospec-review/opencode/agent.md
```

- [ ] **Step 3: Build codex/prompt.md (no frontmatter, leading blank line for diff parity)**

```bash
{ echo ""; cat /tmp/autospec-review-body.md; } > skills/autospec-review/codex/prompt.md
```

- [ ] **Step 4: Verify lock-step**

```bash
diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
     <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/opencode/agent.md)
diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
     <(cat skills/autospec-review/codex/prompt.md | sed '1{/^$/d;}')
```
Expected: both diffs empty.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-review/opencode/agent.md skills/autospec-review/codex/prompt.md
git commit -m "feat(autospec-review): lock-step opencode + codex variants"
```

---

## Task 17: Write skill body lock-step bats test

**Files:**
- Create: `tests/test_autospec_review_skill_lockstep.bats`

- [ ] **Step 1: Copy autospec/test_<...>_lockstep.bats as a model**

```bash
ls tests/*lockstep*.bats | head -3
cat tests/test_phase4_guardian_trio.bats | head -40
```

- [ ] **Step 2: Write the new bats test**

```bash
#!/usr/bin/env bats
# Verify autospec-review skill body is byte-identical across SKILL.md,
# opencode/agent.md, and codex/prompt.md.

@test "skill body byte-identity: SKILL.md vs opencode/agent.md" {
  diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
       <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/opencode/agent.md)
}

@test "skill body byte-identity: SKILL.md vs codex/prompt.md" {
  diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
       <(sed '1{/^$/d;}' skills/autospec-review/codex/prompt.md)
}
```

- [ ] **Step 3: Run the bats test — expect PASS**

```bash
bats tests/test_autospec_review_skill_lockstep.bats
```

- [ ] **Step 4: Commit**

```bash
git add tests/test_autospec_review_skill_lockstep.bats
git commit -m "test(autospec-review): skill body lock-step bats"
```

---

## Task 18: Modify autospec-run/SKILL.md — queue priority sort

**Files:**
- Modify: `skills/autospec-run/SKILL.md`

- [ ] **Step 1: Locate the existing queue-sort section in autospec-run**

```bash
grep -n "auto-implement" skills/autospec-run/SKILL.md | head
grep -n "oldest" skills/autospec-run/SKILL.md | head
```

Identify the lines that describe the current queue ordering (likely a
"Queue:" or "Issue selection:" subsection in Phase 4).

- [ ] **Step 2: Insert a new subsection above the existing sort logic**

The new content (insert exactly):

```markdown
### Queue priority sort (autospec-review interlock)

When selecting the next `auto-implement` issue, sort:

1. First: issues with label `priority:high` (e.g. `[REGRESSION]`
   issues filed by autospec-review). Within `priority:high`, oldest
   first.
2. Then: all other `auto-implement` issues, oldest first.

`priority:high` always wins over age. This guarantees regression
issues unblock the queue before continuing with normal feature work.
```

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-run/SKILL.md
git commit -m "feat(autospec-run): queue priority sort for priority:high"
```

---

## Task 19: Modify autospec-run/SKILL.md — Tier-A LGTM escalation block

**Files:**
- Modify: `skills/autospec-run/SKILL.md`

- [ ] **Step 1: Locate the existing LGTM reviewer dispatch in `process(ISSUE)`**

```bash
grep -n "LGTM" skills/autospec-run/SKILL.md
grep -n "**Model tier:**" skills/autospec-run/SKILL.md
```

Find the line/block that dispatches the LGTM reviewer at Tier B.

- [ ] **Step 2: Wrap that dispatch with a label-conditional**

Insert the new conditional immediately before the existing LGTM
dispatch:

```markdown
### Regression review escalation

If the issue's labels include `regression` OR `priority:high`:

- **Model tier:** Tier A (regression review) — top model + ultrathink.
- Run TWO reviewer passes in sequence:

  **Pass 1.** Standard LGTM judgment. If FAIL, return to implementer
  with directives.

  **Pass 2.** Meta-review prompt:
  > Would the Implementation Guardian or this LGTM reviewer have
  > caught the original gap during the first implementation? If yes,
  > what review questions failed? Add the missing checklist items to
  > `reports/autospec-review/reviewer-lessons.md` (one entry per item,
  > with parent gap_id and date) and re-review with the augmented
  > checklist.

- Both passes must approve before merge.

Otherwise (default path):

- **Model tier:** Tier B (implementation work).
- Single LGTM pass.
```

- [ ] **Step 3: Verify the existing default LGTM block now sits inside the "Otherwise" branch**

```bash
grep -B 2 -A 10 "Otherwise (default path)" skills/autospec-run/SKILL.md
```

- [ ] **Step 4: Commit**

```bash
git add skills/autospec-run/SKILL.md
git commit -m "feat(autospec-run): Tier-A LGTM escalation + 2-pass on regression issues"
```

---

## Task 20: Modify autospec-run/SKILL.md — Phase 6 post-batch trigger

**Files:**
- Modify: `skills/autospec-run/SKILL.md`

- [ ] **Step 1: Locate the end of the existing outer-loop / final-report section**

```bash
grep -n "^## Phase" skills/autospec-run/SKILL.md
grep -n "Final" skills/autospec-run/SKILL.md
```

Append the new phase AFTER the existing final phase.

- [ ] **Step 2: Append Phase 6**

```markdown
## Phase 6 — Post-batch audit (autospec-review interlock)

Runs after the last issue in this batch closes/merges, before printing
the final report.

Skip when:

- `~/.autospec/no-review.flag` exists, OR
- `--no-postreview` was passed to autospec-run.

Otherwise:

```bash
/autospec-review --since "${BATCH_START_DATE}"
```

On gaps found: post a comment to the autospec-run status thread
summarising gap counts by spec. Do NOT block batch completion.
Failures from `/autospec-review` log a warning but do not fail the
overall run.
```

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-run/SKILL.md
git commit -m "feat(autospec-run): Phase 6 post-batch /autospec-review trigger"
```

---

## Task 21: Lock-step autospec-run variants

**Files:**
- Modify: `skills/autospec-run/opencode/agent.md`
- Modify: `skills/autospec-run/codex/prompt.md`

- [ ] **Step 1: Extract updated SKILL.md body**

```bash
awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md > /tmp/autospec-run-body.md
```

- [ ] **Step 2: Re-build opencode variant (preserve frontmatter)**

```bash
# Capture existing frontmatter
awk '/^---$/{c++; print; next} c<2{print}' skills/autospec-run/opencode/agent.md > /tmp/autospec-run-opencode-fm.md
cat /tmp/autospec-run-opencode-fm.md /tmp/autospec-run-body.md > skills/autospec-run/opencode/agent.md
```

- [ ] **Step 3: Re-build codex variant (no frontmatter; leading blank line)**

```bash
{ echo ""; cat /tmp/autospec-run-body.md; } > skills/autospec-run/codex/prompt.md
```

- [ ] **Step 4: Verify lock-step**

```bash
diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md) \
     <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/opencode/agent.md)
diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md) \
     <(sed '1{/^$/d;}' skills/autospec-run/codex/prompt.md)
```
Expected: both empty.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/opencode/agent.md skills/autospec-run/codex/prompt.md
git commit -m "feat(autospec-run): lock-step opencode + codex variants"
```

---

## Task 22: Bats lock-step tests for autospec-run blocks

**Files:**
- Create: `tests/test_autospec_run_priority_sort_lockstep.bats`
- Create: `tests/test_autospec_run_regression_review_lockstep.bats`

- [ ] **Step 1: Write priority-sort byte-identity bats**

```bash
#!/usr/bin/env bats
@test "priority sort block exists in SKILL.md" {
  grep -q "Queue priority sort" skills/autospec-run/SKILL.md
}
@test "priority sort block byte-identical: SKILL.md vs opencode" {
  diff <(sed -n '/Queue priority sort/,/^---/p' skills/autospec-run/SKILL.md | head -20) \
       <(sed -n '/Queue priority sort/,/^---/p' skills/autospec-run/opencode/agent.md | head -20)
}
@test "priority sort block byte-identical: SKILL.md vs codex" {
  diff <(sed -n '/Queue priority sort/,/^---/p' skills/autospec-run/SKILL.md | head -20) \
       <(sed -n '/Queue priority sort/,/^---/p' skills/autospec-run/codex/prompt.md | head -20)
}
```

- [ ] **Step 2: Write regression-review byte-identity bats** (mirror, replacing `Queue priority sort` with `Regression review escalation`)

- [ ] **Step 3: Run both bats files — expect all green**

```bash
bats tests/test_autospec_run_priority_sort_lockstep.bats
bats tests/test_autospec_run_regression_review_lockstep.bats
```

- [ ] **Step 4: Commit**

```bash
git add tests/test_autospec_run_*_lockstep.bats
git commit -m "test(autospec-run): lock-step bats for priority sort + regression review"
```

---

## Task 23: Extend scripts/validate.sh with 4 new checks

**Files:**
- Modify: `scripts/validate.sh`

- [ ] **Step 1: Locate the existing check_* function pattern**

```bash
grep -n "^check_" scripts/validate.sh | head
```

- [ ] **Step 2: Append four new functions**

```bash
check_autospec_run_priority_sort_lockstep() {
  diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md | grep -A 8 'Queue priority sort') \
       <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/opencode/agent.md | grep -A 8 'Queue priority sort') \
    || { echo "FAIL: priority sort lockstep (opencode)"; return 1; }
  diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-run/SKILL.md | grep -A 8 'Queue priority sort') \
       <(sed '1{/^$/d;}' skills/autospec-run/codex/prompt.md | grep -A 8 'Queue priority sort') \
    || { echo "FAIL: priority sort lockstep (codex)"; return 1; }
}

check_autospec_run_regression_review_lockstep() {
  for variant in "skills/autospec-run/opencode/agent.md" "skills/autospec-run/codex/prompt.md"; do
    grep -q "Regression review escalation" "$variant" \
      || { echo "FAIL: regression review block missing in $variant"; return 1; }
    grep -q "Tier A (regression review)" "$variant" \
      || { echo "FAIL: Tier A annotation missing in $variant"; return 1; }
  done
}

check_autospec_review_skill_present() {
  for f in "skills/autospec-review/SKILL.md" \
           "skills/autospec-review/opencode/agent.md" \
           "skills/autospec-review/codex/prompt.md"; do
    [ -f "$f" ] || { echo "FAIL: $f missing"; return 1; }
  done
  diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
       <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/opencode/agent.md) \
    || { echo "FAIL: autospec-review opencode lockstep"; return 1; }
  diff <(awk '/^---$/{c++; next} c>=2' skills/autospec-review/SKILL.md) \
       <(sed '1{/^$/d;}' skills/autospec-review/codex/prompt.md) \
    || { echo "FAIL: autospec-review codex lockstep"; return 1; }
}

check_autospec_review_tier_a_directives() {
  grep -q "Tier A (spec work)" skills/autospec-review/SKILL.md \
    || { echo "FAIL: missing 'Tier A (spec work)' directive"; return 1; }
  # at least 2 occurrences (audit subagent + reviewer subagent)
  count=$(grep -c "Tier A (spec work)" skills/autospec-review/SKILL.md)
  [ "$count" -ge 2 ] \
    || { echo "FAIL: expected ≥2 'Tier A (spec work)' directives, found $count"; return 1; }
}
```

- [ ] **Step 3: Wire the new functions into the existing check runner**

Find where existing checks are called (typically a `main()` or
end-of-file loop) and append:

```bash
check_autospec_run_priority_sort_lockstep
check_autospec_run_regression_review_lockstep
check_autospec_review_skill_present
check_autospec_review_tier_a_directives
```

- [ ] **Step 4: Run validate.sh — expect ALL PASS**

```bash
bash scripts/validate.sh
```

- [ ] **Step 5: Commit**

```bash
git add scripts/validate.sh
git commit -m "feat(validate): autospec-review and autospec-run regression checks"
```

---

## Task 24: Update SKILLS.md and top-level README.md

**Files:**
- Modify: `SKILLS.md`
- Modify: `README.md`

- [ ] **Step 1: Add autospec-review row to SKILLS.md**

Find the existing skills table and add:

```markdown
| autospec-review | `/autospec-review` | Audit specs vs issues; file `[REGRESSION]` issues with `priority:high` |
```

- [ ] **Step 2: Update top-level README.md suite list**

Find the bulleted suite list and add:

```markdown
- **autospec-review** — close the spec-vs-code feedback loop; finds gaps, files high-priority regression issues
```

- [ ] **Step 3: Commit**

```bash
git add SKILLS.md README.md
git commit -m "docs: register autospec-review in SKILLS.md and README"
```

---

## Task 25: End-to-end validate.sh + bats run

**Files:** none (verification only)

- [ ] **Step 1: Run all validation in one go**

```bash
bash scripts/validate.sh && bats tests/*.bats && pytest tests/test_autospec_review.py -v
```

Expected: all green. Any failure means the task that introduced the
diff was incomplete; go back and fix before continuing.

---

## Task 26: File the `/autospec-split --no-bracketing` follow-up issue

**Files:** none

- [ ] **Step 1: Open follow-up issue in autospec repo**

```bash
gh issue create \
  --repo berlinguyinca/autospec \
  --title "autospec-split: --no-bracketing flag for regression specs" \
  --body "Tracked by autospec-review design (docs/specs/2026-05-06-autospec-review-design.md §6 + §12).

When autospec-review hands a regression spec to /autospec-split, the
default Pre-work N.0 + Review N.review sandwich is over-structured —
regression issues are already concrete fixes. Add a --no-bracketing
flag /autospec-review can pass.

Acceptance:
- [ ] /autospec-split honors --no-bracketing
- [ ] when set, child issues skip the bracketing pre-work and review steps
- [ ] autospec-review SKILL.md updated to pass --no-bracketing
- [ ] tests cover both bracketing-on and bracketing-off paths
" \
  --label "auto-implement"
```

- [ ] **Step 2: Note the issue number for the autospec-review SKILL.md follow-up**

(Tracked by autospec-run picking it up via auto-implement label.)

---

## Task 27: Update PR #240 with implementation status

**Files:** none

- [ ] **Step 1: Push the implementation commits**

```bash
git push origin feat/autospec-review
```

- [ ] **Step 2: Add a PR comment summarising the implementation pass**

```bash
gh pr comment 240 --body "Implementation complete on this branch:
- skill: \`skills/autospec-review/\` (SKILL.md + lock-step variants + 4 reference docs + 1 template)
- script: \`scripts/autospec_review_audit.py\` with full pytest coverage
- autospec-run modifications: queue priority sort, Tier-A LGTM escalation + 2-pass, Phase 6 post-batch trigger (all lock-step)
- 4 new validate.sh checks; 3 new bats lock-step tests
- follow-up issue filed for /autospec-split --no-bracketing

Ready for review."
```

---

## Self-review checklist (executor must run before claiming done)

- [ ] **Spec coverage:** every section of `docs/specs/2026-05-06-autospec-review-design.md` (1-12) maps to at least one task above. Missing? Add a task.
- [ ] **Placeholder scan:** `grep -nE "TBD|TODO|FIXME|XXX|placeholder" docs/plans/2026-05-06-autospec-review.md` returns nothing.
- [ ] **Type consistency:** `compute_gap_id`, `discover_specs`, `link_issues`, `validate_subagent_output`, `write_per_run_csv`, `merge_into_ledger`, `generate_run_id`, `gh_issue_list` — all referenced in tasks 14–15 with the names defined in tasks 8–13.
- [ ] **Lock-step:** every change to a SKILL.md has a paired Task that also touches opencode + codex AND is asserted by validate.sh.
- [ ] **TDD discipline:** every script-touching task starts with a failing test.

If any checkbox above stays unchecked, the task that introduced the
gap is the one to revisit.
