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
