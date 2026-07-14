# Autospec reviewer lessons

- date: 2026-07-09
  issue: 1622
  gap_id: control-plane-validation-enumeration
  lesson: Priority/high release audits must compare each dependency issue's primary smoke test against the repo-wide validation harness, not only verify that the focused test file exists.
  applied_check: When an integration epic closes, enumerate child issue smoke commands and require `autospec validate` to run every release-critical smoke or document a deliberate exclusion.
- date: 2026-07-09
  issue: 1627
  gap_id: all-blocked-backlog-livelock
  lesson: Priority/high queue reviews must distinguish an empty backlog from a non-empty backlog whose candidates are all blocked by dependency edges.
  applied_check: When readiness changes touch dependency parsing, require regression coverage for non-blocking umbrella edges, true sibling blockers, cycle reporting, and conductor all-blocked escalation.
