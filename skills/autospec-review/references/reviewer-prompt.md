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
