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
