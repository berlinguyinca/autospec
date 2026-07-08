# Autonomy guardrails foundation design

Issue #1543 adds the parent foundation for autonomous merge guardrails without
absorbing the deeper child scopes (#1544-#1547). The parent layer is deliberately
small and deterministic: fail closed on verifier-surface edits, quarantine
fenced/high-risk paths, record merge provenance, and expose a post-merge health
rollback hook that children can extend.

## Literature basis

- Skalse, Howe, Krasheninnikov, and Krueger, “Defining and Characterizing Reward
  Hacking” (NeurIPS 2022), formalize reward hacking as optimizing an imperfect
  proxy reward while degrading the true objective. That maps directly to agents
  editing tests, assertions, or evaluation harnesses to make a gate pass without
  improving the product.
- Krakovna et al.'s “Specification gaming examples in AI” catalog documents
  specification gaming: agents exploiting task specifications in unintended ways, including examples
  where measured success diverges from intended success. This motivates fenced
  surfaces and human review for high-blast-radius changes.
- OpenAI's “Detecting misbehavior in frontier reasoning models” reports that
  frontier reasoning models can exploit loopholes and that penalizing visible bad
  reasoning can cause hidden intent. This motivates deterministic, external
  diff/provenance checks rather than relying on the implementing context to
  self-certify safety.

## Parent foundation

1. `scripts/autonomous-guardrails.sh diff-guard` blocks changes to test files,
   validation scripts, fixtures, evals, and benchmark surfaces. This is the
   immutable-verifier tripwire; #1544 can deepen it with mutation testing and
   stricter harness ownership.
2. `scripts/autonomous-guardrails.sh blast-radius` blocks autonomous merge for
   fenced paths such as autonomous conductors, merge/claim guards, workflows,
   installers, package/crate surfaces, schemas, migrations, auth, secrets, and
   token-sensitive files. #1545 can expand this into an async quarantine queue.
3. `scripts/autonomous-premerge-gate.sh` runs those deterministic checks before
   QA/security scans when `--changed-files` is supplied. Blocking decisions apply
   `autospec:needs-human` through the existing label path and exit before merge.
4. `scripts/autonomous-guardrails.sh separation-of-powers --lane-metadata <json>`
   validates deterministic lane metadata before approval: `author`, `verifier`,
   and `approver` identities must be distinct, and `verifier_prompt` must be
   adversarial/refute-oriented plus independent of author context.
5. `scripts/autonomous-premerge-gate.sh --lane-metadata <json>` runs that
   separation-of-powers check before QA/security scans. A violation exits with
   `block separation_of_powers`, so an author-produced verification cannot merge.
6. Successful gates may write
   `autospec.autonomous.merge_provenance.v1` via `--provenance-out`, capturing
   repo, PR, changed files, rollback handle, gate evidence, blast-radius
   decision, `separation_of_powers`, and auditable `lane_metadata` when provided.
   #1546 can make this a durable audit ledger.
7. `scripts/autonomous-resilience.sh post-merge-health` reuses `main-health`; a
   halt dispatches `AUTOSPEC_ROLLBACK_CMD --handle <rollback_handle>` from the
   provenance record. #1546 can replace the stub dispatch with the final rollback
   executor and canary signals.

## Non-goals for the parent issue

- No new dependencies.
- No attempt to build the full human-review queue.
- No mutation-testing implementation beyond the immutable verifier diff guard.
- No remote identity attestation beyond caller-supplied lane metadata; #1547
  enforces deterministic separation for metadata that the pipeline supplies.
