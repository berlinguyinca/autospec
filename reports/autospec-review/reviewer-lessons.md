# Autospec reviewer lessons

- date: 2026-07-09
  issue: 1622
  gap_id: control-plane-validation-enumeration
  lesson: Priority/high release audits must compare each dependency issue's primary smoke test against the repo-wide validation harness, not only verify that the focused test file exists.
  applied_check: When an integration epic closes, enumerate child issue smoke commands and require `scripts/validate.sh` to run every release-critical smoke or document a deliberate exclusion.
