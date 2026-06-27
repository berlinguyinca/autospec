---
name: feedback_explore_once_unverified_near_zero_precision
description: autospec-explore local discovery on the autospec repo produced 183 raw candidates and 0 verified filing candidates; never auto-file --once output unverified
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 37d298e0-f75e-4f7a-8457-db86f85c4404
---

A local-tier `/autospec-explore` discovery pass on the autospec repo itself (10
sources, no internet, `--stage dedup`) produced **183 raw deduped proposals** that
collapsed to **0 confidently file-worthy** after file-level verification:

- `quality-resilience` (100) and `self-leverage` (50) both saturated at their per-round
  caps — a saturation signal, not signal. `self-leverage` grepped *prose in spec docs*
  ("operator must invoke --resume explicitly so the audit trail is reviewed") and
  proposed auto-resolving deliberate human-in-loop safety gates (dangerous false positives).
- `codebase-signals` (24): the known TODO/FIXME-grep noise class ([[feedback_explore_codebase_signals_false_positives]]).
- `spec-vs-code` (1): matched the template placeholder `{{ac_bullets_from_taxonomy}}` as an "unimplemented AC."
- `source-analysis` (5) was the *best* source — yet on direct evidence-checking, 4/4 of its
  filable proposals were **factually false or already-resolved**: README already documents
  autospec-autonomous (claimed missing); conductor bats tests already exist (claimed absent);
  install.sh already bundles scripts/lib + `tests/ship-completeness.bats` already guards it
  (claimed missing — the [[feedback_installer_excludes_runtime_libs]] bug is already fixed).

**Why:** The `--once` path's adversarial-verify stage degrades to `no-op-unverified` when
run as a bare bash subprocess (no LLM skeptic wired in). Filing unverified `--once`
output as `auto-implement` would flood `main` with noise — and even the "grounded"
sources carry false evidence that only direct file inspection catches.

**How to apply:** When a conductor escalates to Tier-2 discovery, do NOT run
`/autospec-explore --once` as a bare subprocess and auto-file. Instead run `--stage dedup`
to get candidates, then have the harness LLM (you) serve as the adversarial skeptic —
**verify each survivor's evidence against the actual files, refute-by-default** — before
filing anything. A 0-verified pass is a legitimate, honest outcome: report it and keep the
backlog empty rather than manufacturing work. Relates to [[feedback_per_pr_lgtm_misses_integration]]
(verification catches what generators miss) and [[feedback_roi_check_new_components]].
