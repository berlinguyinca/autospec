### Added

- `scripts/lint-issue.sh` — new `SCOPE_HEDGE` rule: scope sections (`## Goal`,
  `## Acceptance criteria`, `## Implementation outline`) may not carry hedged
  or interim phrasing (`interim`, `for now`, `at minimum`, `ideally`,
  `conservative`), because an agent implements the smallest thing the spec can
  be read as permitting; such phrasing belongs in a rationale section or the
  smaller thing becomes its own issue (#4275, 2026-09-10).
