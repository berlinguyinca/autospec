# Local model guardrails — analysis and rule set (tracker #3344)

**Goal:** Keep local models (Qwen 3.8 27B and peers) out of planning and review
lanes, and make local implementation dispatch fail closed on the failure modes
operators actually report — without re-litigating the routing surface that
already exists.

**Doctrine:** a bigger model plans, the local model implements. The local
model's one preferred lane is vision QA, where frontier models have no eyes.

This is the tracker's analysis document. The children it sequences implement
the rules below one at a time; nothing here is done until the blocking decision
point runs.

> **Numbering note:** the R-numbers in this document are scoped to this
> tracker. They are unrelated to the workstream numbering in
> [`docs/specs/2026-08-05-self-discovering-model-routing-design.md`](../specs/2026-08-05-self-discovering-model-routing-design.md)
> (whose R1 "local executor" is already shipped as `scripts/local-dispatch.sh`).

---

## 1. The evidence

Operators reported ten distinct local-model failure modes. They converge to
three classes, plus two secondary ones that only matter once the first three
are contained:

1. **Reasoning loops that never emit.** At medium effort the model philosophizes
   over a failed `git push` for a quarter of an hour and produces no artifact.
   Cost with no output — invisible to a token ledger that only counts what was
   spent, not what was produced.
2. **Repo-wide planning collapses with complexity.** The model plans fine on a
   one-file change and falls apart once the change spans modules. Planning
   quality is the upstream bottleneck: a cheap error here costs N implementer
   cycles downstream.
3. **Agent tool loops hang while raw generation stays fine.** Benchmark scores
   measure the model generating text, not the model driving a tool loop — and
   it is the loop that hangs. Any eligibility decision made from a
   generation-quality score alone is measuring the wrong thing.

Secondary, contained by the rules below rather than by routing:

4. **Weak structured output** — the model's JSON/YAML conformance degrades
   faster than its prose (R7).
5. **Unprompted unsafe initiative** — hunting the filesystem for credentials to
   install a package nobody asked it to install (R9).

## 2. What already exists

The guardrails land on a routing surface that is already built and already
encodes part of the doctrine:

- **`scripts/route-decide.sh`** — the decision layer. Its overridable set is an
  **allowlist** (`OVERRIDABLE_KINDS` at `route-decide.sh:143`:
  `implementer explore-researcher refine-lens qa-sweep`), not a blocklist: any
  kind not on it — including kinds added to the ledger vocabulary later — falls
  through to the baseline. Five kinds are deliberately absent, each with its
  reason inline at `route-decide.sh:19-30`: `lgtm-reviewer` (a cheap model
  reviewing its own tier's output degrades quality invisibly, and the ledger
  would record that as a first-pass success), `verify-voter` (voter
  independence is a vendor question, not a cost one), `secaudit-pass` (safety
  gate; never local, never downgraded), `spec-decompose` (spec quality is the
  upstream bottleneck), `growth-lens` (unproven against a ledger).
- **`scripts/verify-voter-vendor.sh`** — enforces that a verify voter comes
  from a vendor *different* from the proposer's, and fails closed (exit 3)
  when no independent vendor is available. Vendor is an independence lever;
  tier is a quality lever; they are not the same decision.
- **`scripts/local-dispatch.sh`** — the fail-closed local executor. Three
  preconditions checked in order (Codex CLI present *and* advertising `--oss`;
  capability probe reports `dispatch_recommended`; wall-clock ceiling, default
  600 s), a capacity-1 host lock (one GPU cannot serve two dispatches), and
  exit codes that hand the decision back to the caller (3 → keep the cloud
  tier, 4 → ceiling exceeded).

What is missing is not the rules — it is the wiring: **`route-decide.sh` has no
executable caller**, and `dispatch-implementer.sh` consults no routing helper.
The allowlist is documented behaviour that nothing executes.

## 3. Blocking decision point

**#3179** wires the decision point in **shadow (log-only) form first**: every
dispatch logs what `route-decide.sh` would have decided before
`dispatch-implementer.sh` acts on it. Every child below assumes the decision
point runs. Nothing in this tracker is real until that lands.

## 4. The rule set

| Rule | Statement | Owner |
|---|---|---|
| **R1** | The non-overridable allowlist is frozen and one-way-extendable, and extends to planning kinds: every new dispatch kind is non-overridable until this tracker's owner adds it with a written reason. | #3345 |
| **R2** | Role-aware model profiles: every profile carries a `roles:` map (the multi-model spec's §3 roles, orthogonal to `dispatch_kind`), and routing fails closed when a profile lacks the key. | #3346 |
| **R3** | Lane assignment policy: Claude plans, Codex reviews, local implements; vision QA is the local model's one preferred lane. Policy, not code — R1 and R2 encode it. No separate implementation. | — |
| **R4** | Reviewer vendor must differ from author vendor — not just for verify voters, but for every review lane. | #3347 |
| **R5** | No-progress abort and within-run demotion: a local dispatch that has made no artifact progress in N tool iterations is aborted and the issue is demoted back to the cloud tier for the rest of the run. | #3348 |
| **R6** | Triviality floor, and wall-clock in effective cost: below a triviality floor local dispatch is not worth the ceiling, and effective cost counts wall-clock, not just tokens. | #3349 |
| **R7** | Schema validator required for structured output: a local dispatch that must emit JSON/YAML is gated on schema validation, not on the model's confidence. | #3350 |
| **R8** | Verbatim-anchor verification: quoted evidence in a local dispatch's output must be verifiable verbatim against the source, or the output is rejected. | #3351 |
| **R9** | Scrub credentials from prompts and deny package installs: a local dispatch may not read credential stores or install packages; prompts handed to a local runtime are scrubbed of secrets first. | #3352 |
| **R10** | Quant + chat-template fingerprint as the ledger key: benchmark rows and routing evidence are keyed by the exact quantization and chat template, not by model name alone. | #3353 |
| **R11** | Per-stack eligibility, default deny: a model×quant×template stack is eligible for a lane only if evidence says so; absence of evidence means ineligible. | #3354 |
| **R12** | Split generation-quality and agent-loop-completion scores: a benchmark score never conflates what the model generates with whether its tool loop completes. | #3355 |

Rationale, compressed:

- **R1** makes the doctrine structural. The existing allowlist already keeps
  `spec-decompose` and the review kinds off the overridable set with written
  reasons; freezing it means a future kind defaults to *not* routable — the
  same allowlist-not-blocklist logic the decision layer already states.
- **R2** gives the doctrine a data model. A profile without a `roles:` map
  cannot prove it may take a lane, so it fails closed.
- **R5** and **R6** attack failure class 1 directly: no-progress abort caps the
  cost of a philosophizing loop; the triviality floor stops paying the
  dispatch overhead for work too small to justify it; wall-clock-in-cost stops
  the ledger from calling a slow local run "cheap" because it burned few
  tokens.
- **R7** and **R8** are output-quality gates that do not need routing evidence:
  they check what the local dispatch actually produced, this run.
- **R10**, **R11**, **R12** are telemetry-dependent: they need a ledger worth
  keying and scores worth splitting, so they sequence after the measurement
  work.

## 5. Sequencing

Waves; each wave assumes the previous one landed:

| Wave | Issues | Depends on |
|---|---|---|
| 0 | #3179 — shadow decision point | — |
| 1 | #3345 (R1 allowlist), #3346 (R2 roles) | Wave 0 |
| 2 | #3348 (R5 abort/demote), #3349 (R6 floor + wall-clock), #3352 (R9 scrub/deny), #3350 (R7 schema gate), #3351 (R8 anchors) | Wave 1 |
| 3 | #3353 (R10 ledger key), #3354 (R11 per-stack deny), #3355 (R12 score split), #3347 (R4 reviewer vendor) | Wave 2 + telemetry |

R3 needs no separate implementation — it is the policy R1 and R2 encode.

## 6. Non-goals

- No new ledger format, no per-model config switch matrix, no bespoke HTTP
  client: `scripts/local-dispatch.sh` goes through Codex CLI's native local
  support on purpose.
- No re-litigating the five deliberately-absent kinds' written reasons; R1
  freezes them.
- No local planning or review lane, including a "cheap review" lane. The
  ledger rewards what it measures; measure review quality before routing it.
