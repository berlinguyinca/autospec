# Immutable verifier design (issue #1544)

Issue #1544 extends the autonomy guardrails foundation with two deterministic merge blockers:

- `scripts/autonomous-guardrails.sh diff-guard --lane implementer|verifier --changed-files FILE` treats test and eval-harness paths as immutable for the implementer lane. The verifier lane is the only bypass and emits `REASON:verifier_lane_bypass` so the bypass is explicit in gate logs.
- `scripts/autonomous-guardrails.sh mutation-guard --baseline JSON --current JSON` compares mutation scores from deterministic ledgers. If the current score is lower than baseline, it blocks with `REASON:mutation_score_regression` and prints each surviving mutant as `MUTANT:<id>:<file>:<line>:<description>`.

`scripts/autonomous-premerge-gate.sh` accepts `--lane`, `--mutation-baseline`, and `--mutation-current` and runs both guards before QA/secaudit. The mutation inputs intentionally support fixture JSON so validation does not require a real `cargo-mutants` installation.
