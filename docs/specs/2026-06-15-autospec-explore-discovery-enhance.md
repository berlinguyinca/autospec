# autospec-explore — discovery-quality enhancement (verification, dogfooding, self-leverage)

## Summary

`/autospec-explore` already runs a perpetual research → ship loop from 7
deterministic/LLM researchers, aggregates by `confidence × source_weight /
complexity`, caps top-N, and drains via `/autospec-run` onto a sandbox branch.
This spec **extends** that pipeline — it does not replace it — to raise the
quality and breadth of what the loop discovers and to cut the false-positive
rate that is the skill's known failure mode (the constitution gate has dropped
0/45 on a flood of low-signal `codebase-signals` hits).

Four additive enhancements:

1. **Three new research sources** — `quality-resilience`, `dogfooding`,
   `self-leverage` — that surface QA/stability/operability gaps the current 7
   sources are blind to (tests that can't fail, kill-mid-run corruption,
   points where the operator is still a manual bottleneck).
2. **An adversarial verification stage** in the aggregator: every surviving
   proposal is handed to an independent skeptic prompted to *refute* it; only
   survivors are ranked. This is the primary false-positive lever — today
   ranking has no refutation step at all.
3. **Severity + ROI in the proposal contract** — severity is weighted by
   blast radius through auto-merge + lock-step (a silent-wrong-but-green defect
   outranks any missing feature), and every proposal must name a consumer who
   benefits today (ROI gate) or it is dropped.
4. **Pattern synthesis** — before filing, recurring findings (≥2 instances of
   one class, including recurring `docs/memory/` themes) are clustered into a
   single *structural-fix* proposal rather than N point patches.
5. **Domain-specialist researchers (self-discovery + operator selection)** —
   the researcher roster is no longer a fixed list. On top of the universal
   base set, the skill detects the repo's domain(s) and runs a dynamic roster
   of specialist personas appropriate to it (e.g. a quantitative-trading repo
   gets a `quant-strategy`, `market-risk`, and `exchange-integration`
   specialist; a healthcare app gets `hipaa-compliance` and `clinical-safety`).
   The operator can let the skill auto-discover the roster, be asked to confirm
   or name it, or specify it explicitly — and choose how many specialists run.

> **Researcher accounting.** The baseline is **7 universal researchers**
> (`spec-vs-code`, `prior-reports`, `codebase-signals`, `open-issues`,
> `source-analysis`, `dependency-health`, `internet` — corrected from the stale
> "6" the trio prose carried before this spec) **+ 3 discovery researchers**
> (enhancement 1) **+ N domain specialists** (enhancement 5). The aggregator
> already tolerates arbitrary `source` names via `SRC_WEIGHTS.get(src, 0.5)`,
> so specialists slot in without changing the dedup/verify/rank machinery.

The reusable operator-facing form of this same methodology ships as a runbook
at `docs/runbooks/discovery-sweep.md`; this spec and that runbook are kept in
sync (both describe the same five tracks + verify + synthesis).

## Team personality

- **Selected team:** Core product + reliability engineering — product manager,
  reliability/test engineer, backend developer, security advisor, technical
  writer.
- **Why this team fits:** the enhancement is mostly about *quality of
  discovery* — separating real defects from plausible noise, and weighting
  blast radius under autonomous auto-merge. Reliability and test judgment
  dominate; the security advisor owns the live-state and dogfooding read paths.
- **Risks this team will notice:** the verification stage becoming a rubber
  stamp; severity inflation; dogfooding reading machine-specific state that
  breaks portability; new researchers re-flooding the queue with the same
  low-signal noise the codebase-signals source already produces.
- **Carry into child issues:** verification defaults to *refuted* under
  uncertainty; every proposal needs a named consumer; the new researchers
  obey the same JSON contract and per-round caps as the existing 7; live-state
  reads degrade gracefully to empty when the paths are absent.

## Review counter-team

- **Selected counter-team:** Reliability + portability + false-positive audit.
- **What this team should challenge:** does the skeptic pass actually kill
  bad proposals, or does it pass everything? Does severity weighting ever let
  a real auto-merge-blast-radius defect get out-ranked by a shiny feature?
  Does `dogfooding` hard-depend on `~/.autospec/` existing, and does it leak
  host-specific paths into issue bodies? Do the three new researchers add
  signal, or just more volume to dedup?

## Architecture

```
research cycle (each round)
        │
   ┌────┴───────────────────────────────────────────────┐
   │  10 researchers in parallel (7 existing + 3 new)    │
   │    existing: spec-vs-code, prior-reports,           │
   │      codebase-signals, open-issues, source-analysis,│
   │      dependency-health, internet                    │
   │    NEW:                                              │
   │      quality-resilience  (4 QA lenses)              │
   │      dogfooding          (live ~/.autospec state +  │
   │                           git churn/revert)         │
   │      self-leverage       (human-in-loop points)     │
   └────┬───────────────────────────────────────────────┘
        ▼
   aggregate + dedup            (unchanged)
        ▼
   ADVERSARIAL VERIFY  ← NEW    one skeptic per proposal, refute-by-default;
        │                       drop refuted; attach verdict + reason
        ▼
   ROI gate            ← NEW    drop proposals with no named consumer
        ▼
   PATTERN SYNTHESIS   ← NEW    cluster ≥2-instance classes into one
        │                       structural-fix proposal
        ▼
   rank by SEVERITY-first, ← CHANGED  severity (auto-merge blast radius)
   then confidence×weight/complexity   dominates; old score breaks ties
        ▼
   cap top-N → file issues → /autospec-run  (unchanged)
```

Nothing downstream of "file issues" changes; the sandbox-branch contract,
loop driver, usage-limit recovery, and `/autospec-run` drain are untouched.

## New researcher contracts

Each new researcher is a standalone script under `scripts/explore-research/`,
emits the existing proposal JSON (`source`, `proposals[]` with `title`,
`evidence`, `estimated_complexity`, `confidence`) **plus** the two new fields
`severity` and `named_consumer` (see Proposal contract extension), and obeys a
per-round cap. Each is enable/disable-able via `--research-sources`.

| Researcher | Reads | Cap | Default weight |
|---|---|---|---|
| `quality-resilience` | (a) test files vs their SUT — flags self-consistent fixtures built with the SUT's own derivation expr and assertion-free tests; (b) each claimed invariant in `validate.sh`/SKILL prose vs whether a test AND a guard exist; (c) kill-mid-run / non-idempotent / shared-lock / partial-state hazards; (d) LLM steps that should be deterministic + disproportionate-token phases | 100 candidates per round | 0.95 |
| `dogfooding` | live `~/.autospec/` run-state, failure ledgers, heartbeats, `explore-loop.json`, `run-summary.md`; `git log` churn + revert archaeology; never-invoked skills/flags (dead surface) | last 20 runs / 200 commits | 0.9 |
| `self-leverage` | every point in the trio prose + scripts where a human decision/intervention/relaunch is still required; checks each against the autonomy-scope rule (low-stakes should auto-resolve; only run/defer/refine + destructive-remote reach the operator) | 50 candidates per round | 0.6 |

`quality-resilience` and `dogfooding` rank *above* `codebase-signals` (0.7) by
design — they are grounded in real behavior and concrete invariants, where
`codebase-signals` is grep-of-prose noise.

### Portability + safety of the live-state reads

- `dogfooding` reads `${AUTOSPEC_STATE_DIR:-$HOME/.autospec}`; if the dir or a
  given artifact is absent it emits `{"source":"dogfooding","proposals":[]}`
  and exits 0 — never hard-fails (cf. the installer-excludes-runtime-libs and
  bash 3.2 process-sub gotchas).
- Host-specific absolute paths are redacted to repo-relative or `~/`-relative
  form before any value reaches an issue body.
- No live-state read is interpolated into a `jq test()` regex (cf. the
  jq-regex-metachar-injection finding); use `capture()`/`==`.

## Domain-specialist researchers (self-discovery + operator selection)

The universal + discovery researchers are domain-agnostic. A financial trading
repo wants a quant strategist hunting for missing risk controls; a healthcare
app wants a compliance reviewer. Enhancement 5 adds a **dynamic roster of
specialist researchers** chosen to fit the repo.

### What a specialist is

A specialist is an LLM-persona researcher (Tier A or B) that explores the repo
through a domain lens and emits the same extended proposal JSON as every other
researcher, with `source` = `specialist:<slug>` (e.g. `specialist:market-risk`).
Because the aggregator keys weights by source string with a 0.5 default,
specialists need no aggregator change to participate in dedup, verify, ROI,
synthesis, and ranking. Default specialist weight is 0.6 (between
`open-issues` and `source-analysis`); the ledger then learns per-specialist
weight and refutation rate like any other source.

### Roster discovery (run once at sandbox creation, cached)

1. **Deterministic signal scan** (no LLM): dependency manifests
   (`package.json`, `requirements.txt`, `pyproject.toml`, `go.mod`,
   `Cargo.toml`, `*.csproj`, …), README/AGENTS.md keywords, directory taxonomy,
   and a small domain lexicon. Produces a ranked list of candidate domains with
   evidence (file:line) — never a bare guess.
2. **LLM roster proposal** (one Tier-A dispatch): given the signals, emit
   `{domains[], suggested_specialists[]}` where each specialist has
   `{slug, persona, lens, why, evidence}`. Capped at `--num-specialists`.
3. The roster is written to `.autospec/explore-specialists.json` (schema:
   `schemas/autospec-explore-specialists.schema.json`, new) and reused on every
   subsequent round and re-invocation (idempotent, like the sandbox state).

### Operator selection modes

A new `--specialists-mode` flag controls how the roster is finalized:

| Mode | Behavior |
|---|---|
| `discover` (default) | Auto-run discovery. In an **interactive** harness, present the proposed roster via `AskUserQuestion` for confirm/edit before the first round. In an **autonomous** run (`--autonomous`/no TTY), take the top `--num-specialists` and log them — never block. |
| `ask` | Always ask the operator to name the specialists (and count) up front, seeding the prompt with the discovered suggestions. |
| `explicit` | Use `--specialists <slug:persona,…>` verbatim; skip discovery. |
| `off` | No specialists — universal + discovery researchers only (the current behavior; full backward compatibility). |

New invocation flags (extend the existing `/autospec-explore` flag set):

```
--specialists-mode discover|ask|explicit|off   (default: discover)
--num-specialists N                            (default 3, cap 6)
--specialists <slug:persona,slug:persona,…>    (explicit roster)
```

### Guardrails

- Specialists are **researchers, not implementers** — they only propose; all
  proposals still flow through verify → ROI → severity → `/autospec-run`. A
  domain persona cannot bypass the skeptic stage.
- The total parallel researcher count per round is capped
  (`7 universal + 3 discovery + ≤6 specialists = ≤16`) to bound context/cost;
  `--research-sources` can still subset the universal+discovery set.
- Discovery degrades gracefully: if signal scan finds no domain (generic repo),
  the roster is empty and the loop runs exactly as today.
- Specialist personas are derived from repo evidence only; no external persona
  is injected from the internet researcher's fetched content (trust boundary).

## Proposal contract extension

Each proposal gains two fields, with a schema at
`schemas/autospec-explore-proposal.schema.json` (new):

- `severity` — one of `silent-wrong` > `correctness` > `stability` >
  `operability` > `feature` > `nicety`. `silent-wrong` and `correctness`
  proposals that sit behind auto-merge are top-ranked.
- `named_consumer` — free text naming a skill/workflow/operator step that
  benefits *today*. Empty → dropped by the ROI gate.

Backward compatibility: existing researchers that don't emit the fields get
`severity: feature` and `named_consumer: ""` defaulted by the aggregator (the
latter does **not** auto-drop legacy researchers — only the three new ones are
ROI-gated, to avoid silently muting the existing 7 during rollout).

## Aggregator changes (`scripts/explore-research-cycle.sh`)

1. **Verify stage** (new, between dedup and rank): for each deduped proposal,
   dispatch one Tier-B skeptic subagent prompted "Try to refute this proposal;
   default to refuted=true under uncertainty." Attach `{verdict, reason}`.
   Drop `refuted`. The dispatch reuses the existing researcher-subagent
   harness-adapter mapping; if no subagent capability exists, fall back to a
   single in-thread refutation pass (documented degradation, logged).
2. **ROI gate** (new): drop new-source proposals with empty `named_consumer`.
3. **Pattern synthesis** (new): group survivors by a coarse class key; any
   class with ≥2 members (or matching a recurring `docs/memory/` theme)
   collapses to one `structural-fix` proposal whose evidence lists all
   instances and the single guard that would catch them all.
4. **Severity-first ranking** (changed): primary sort key = severity rank;
   secondary = the existing `confidence × source_weight / complexity`.
5. The aggregator's per-iteration JSON log gains `proposals_after_verify`,
   `proposals_refuted`, `proposals_after_roi`, and `structural_fixes` counts
   so the ledger and `explore-summary.md` can report verification yield.

## Ledger integration

The outcome ledger already learns `source_weight` from which proposals ship
clean. Extend it to also record, per source, the **refutation rate** from the
verify stage, so a source that consistently produces refuted proposals is
down-weighted automatically — closing the loop on false positives without
hand-tuning. No new ledger file; add the field to the existing per-source
record.

## Runbook artifact

Ship `docs/runbooks/discovery-sweep.md` — the operator-runnable, harness-
neutral form of this methodology (Phase 0 calibrate → Phase 1 ground truth →
Phase 2 five-track discovery → 2.5 pattern synthesis → 3 verify+rank → 4 file
issues → 5 integrate → 6 compound to memory). It is the human entrypoint when
someone wants a one-shot sweep without arming the perpetual loop, and it is the
reference the three new researchers + verify stage are derived from. A
`validate.sh` check asserts the runbook and this spec list the same five tracks
(lockstep prose, like the existing area-dispatch lockstep checks).

### The five discovery tracks (lockstep anchor)

The runbook and this spec describe the **same five discovery tracks**; the
`check_autospec_explore_discovery_contract` gate in `scripts/validate.sh`
asserts both documents name all five (plus the verify stage and pattern
synthesis):

- **Track A — Feature delta** — promised/implied capabilities with no working
  implementation (`spec-vs-code`, `source-analysis`).
- **Track B — External/ecosystem** — comparable tools, papers, standards via
  the `internet` researcher; cite source URLs.
- **Track C — Quality & resilience** — the four QA lenses (test-of-tests,
  invariant↔guard coverage, failure-injection, determinism & cost):
  `quality-resilience`, `dependency-health`.
- **Track D — Dogfooding** — live `~/.autospec` run-state, git churn + revert
  archaeology, dead surface: `dogfooding`, `prior-reports`, `open-issues`.
- **Track E — Self-leverage** — every remaining human-in-loop point checked
  against the autonomy-scope rule: `self-leverage`.

## Testing

- `tests/explore/test_explore_quality_resilience.bats` — the 4 lenses each
  emit well-formed proposals from fixtures; assertion-free-test detection and
  invariant↔guard coverage fire on a seeded gap.
- `tests/explore/test_explore_dogfooding.bats` — reads a fixture
  `~/.autospec`-shaped dir; **and** asserts empty-output-exit-0 when the dir is
  absent; asserts host-path redaction.
- `tests/explore/test_explore_self_leverage.bats` — flags a seeded
  human-in-loop point; does not flag an already-auto-resolved one.
- `tests/explore/test_explore_verify_stage.bats` — a seeded bogus proposal is
  refuted and dropped; a seeded real one survives; refute-by-default on the
  ambiguous case.
- `tests/explore/test_explore_severity_roi.bats` — severity-first ordering; a
  no-named-consumer new-source proposal is dropped while a legacy one is kept;
  pattern synthesis collapses a 3-instance class to one structural-fix.
- `tests/explore/test_explore_specialists.bats` — signal scan detects a seeded
  domain (e.g. a fixture repo with `ccxt`/`backtrader` deps → trading) and
  proposes matching specialists; an empty/generic repo yields an empty roster;
  `--specialists-mode off` runs zero specialists; the ≤16 per-round cap holds;
  a `specialist:<slug>` proposal flows through verify + ROI like any source.

## Acceptance

- [ ] Three new researcher scripts ship under `scripts/explore-research/`
      (`quality-resilience.sh`, `dogfooding.sh`, `self-leverage.sh`), each
      emitting the extended proposal JSON and obeying its per-round cap.
- [ ] `dogfooding.sh` reads `${AUTOSPEC_STATE_DIR:-$HOME/.autospec}`, degrades
      to empty-output-exit-0 when absent, and redacts host-specific paths.
- [ ] `scripts/explore-research-cycle.sh` gains the verify stage, ROI gate,
      pattern synthesis, and severity-first ranking, with the four new
      per-iteration log counters.
- [ ] `schemas/autospec-explore-proposal.schema.json` defines `severity` and
      `named_consumer`; legacy researchers default safely.
- [ ] The autospec-explore trio (SKILL.md + codex/prompt.md + opencode/agent.md)
      documents the 10 researchers, the verify/ROI/synthesis stages, and the
      severity model, and passes `check_lockstep` + sha256 goldens.
- [ ] `docs/runbooks/discovery-sweep.md` ships and lists the same five tracks
      as this spec.
- [ ] The outcome ledger records per-source refutation rate and down-weights
      high-refutation sources.
- [ ] `scripts/validate.sh` gains `check_autospec_explore_discovery_contract()`
      enforcing: three new researchers present + bash-valid; aggregator stages
      present; proposal schema present; trio lockstep on the new sections;
      runbook↔spec track lockstep; new bats suites run green.
- [ ] Domain-specialist discovery ships: deterministic signal scan +
      one-shot LLM roster proposal, cached to
      `.autospec/explore-specialists.json`
      (schema `schemas/autospec-explore-specialists.schema.json`).
- [ ] `--specialists-mode discover|ask|explicit|off`, `--num-specialists`, and
      `--specialists` flags work; `discover` confirms via `AskUserQuestion`
      interactively and auto-selects top-N in autonomous runs; `off` reproduces
      current behavior byte-for-byte.
- [ ] Specialists run as `source=specialist:<slug>` researchers through the
      full verify → ROI → synthesis → rank pipeline with default weight 0.6 and
      the ≤16-researcher per-round cap enforced.
- [ ] A repo with no detectable domain yields an empty roster and the loop runs
      unchanged.
- [ ] The trio prose baseline is corrected to **7 universal researchers**
      (the stale "6" is gone) and documents the specialist roster mechanism.
- [ ] All new bats fixtures pass; `bash scripts/validate.sh` is green.

## Decomposition into child issues

Aiming for 6 children plus an umbrella.

1. **Issue A — proposal contract + schema + aggregator defaults**: add
   `severity`/`named_consumer` to the contract, ship the schema, teach the
   aggregator to default legacy proposals safely. No behavior change yet.
   Files: 3.
2. **Issue B — three new researchers**: `quality-resilience.sh`,
   `dogfooding.sh`, `self-leverage.sh` + bats for each + portability/redaction
   tests. Depends on A. Files: 6.
3. **Issue C — verify stage + ROI gate + severity-first ranking** in
   `explore-research-cycle.sh` + bats + the four log counters. Depends on A.
   Files: 2.
4. **Issue D — pattern synthesis + ledger refutation-rate down-weighting** +
   bats. Depends on C. Files: 2.
5. **Issue E — domain-specialist roster (self-discovery + operator selection)**:
   deterministic signal scan + LLM roster proposal,
   `.autospec/explore-specialists.json` + schema, the `--specialists-mode` /
   `--num-specialists` / `--specialists` flags, specialist dispatch as
   `source=specialist:<slug>`, the ≤16 cap, and `AskUserQuestion` confirm in
   interactive mode. Depends on A+C. Files: ~5.
6. **Issue F — trio lockstep + runbook + validate gate + goldens**: update
   SKILL.md/codex/opencode for the 7-universal + 3-discovery + N-specialist
   roster and the new stages, ship `docs/runbooks/discovery-sweep.md`, add
   `check_autospec_explore_discovery_contract()`, regenerate the three sha256
   goldens, e2e bats. Depends on A+B+C+D+E. Files: ~7.

Total: 6 children + 1 umbrella. (The `6→7` researcher-count doc-drift
correction landed ahead of this spec as a standalone working-tree fix; Issue F
carries it forward into the full roster prose.)

## Out of scope (defer to v2)

- A standalone `/autospec-discover` skill (the runbook + enhanced explore
  cover the need; promote to a skill only if a named consumer appears).
- Multi-skeptic (N-vote) verification — single-skeptic refute-by-default for
  v1; escalate to a 3-vote panel only if single-vote false-negatives surface.
- Cross-repo dogfooding (read other repos' `~/.autospec` state).
- Auto-filing structural-fix proposals as epics with child issues (v1 files a
  single issue per structural fix).
```

