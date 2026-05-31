# Cluster: functional-coverage

Scope: every interactive UI surface — controls, forms, validation, dropdowns,
buttons, links — must have a proven behavior and a passing test.

Inputs:
- The generated app's UI source (components, templates, pages).
- The control-intent ledger prompt output (see SKILL.md
  `## Control intent ledger prompt`).
- The user-input-element intent prompt output.

Responsibilities:
- Enumerate every control with `control_id → file:line → intent`.
- For each control, find the test that exercises it; if missing → finding.
- Classify presentational vs interactive (see SKILL.md
  `## Presentational control classification`).
- Defer API contract behavior to `backend-integration`.
- Defer a11y/responsive checks to `accessibility-and-responsive`.

Output JSON shape:
```json
{
  "cluster": "functional-coverage",
  "category": "missing_control_test|invalid_validation|dropdown_empty",
  "control_id": "form.submit",
  "file": "src/components/Foo.tsx:42",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category failing_test` or `missing_function`).

TODO: backfill from `## Control intent ledger prompt` +
`## User input element intent prompt` sections of SKILL.md.
