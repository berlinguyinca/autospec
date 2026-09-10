# Concepts

## Spec

A spec is the durable design artifact AutoSpec writes before implementation. It explains the user goal, constraints, acceptance criteria, files likely to change, and validation approach.

## Spec Metadata

V62+ specs are parsed into a small metadata contract before any dependency ordering or execution queue logic runs. The initial Rust parser reads generated Markdown specs, extracts the title, `## Version`, `## Objective`, `## Dependencies`, `## Files To Create/Modify`, `## Acceptance Criteria`, and `## Validation Commands`, and serializes that shape according to `schemas/autospec-spec-metadata.schema.json`. It also extracts the optional `## Blocking Gates` and `## Run Budget` sections (named blocking gates and the derived run budget); when a spec omits either section the corresponding field serializes as empty, so specs that predate those sections still parse.

The parser is intentionally strict for dependency IDs: dependencies must use generated spec IDs like `v62-rust-core-workspace`. Broader Markdown compatibility is deferred until the generated package path is stable.

## Signal Semantics

A **signal** is any external state a spec reads to make a decision (a count, an exit code, a file's presence, a check's status). A signal that does not declare its semantics is a hole in the spec: a number the reader cannot interpret, because the spec never said what zero, absence, or failure each look like.

Every consumed signal MUST declare three things, in a `## Consumed Signals` section with one `### <signal>` entry per signal:

- `Empty/absent means:` — what zero, empty output, or a missing value says about the world (as opposed to about the measurement).
- `Healthy value:` — the value (or range) the signal takes when the population being observed is healthy.
- `Produced under:` — the conditions under which the signal is produced, so a reader can tell "produced nothing" from "never run".

The spec lint (`autospec lint spec`, rule `CONSUMED_SIGNAL_INCOMPLETE`) is **blocking**: a spec with a consumed-signal entry missing any of the three fields is rejected, and the finding names the signal, the missing field, and the line. This is distinct from the advisory safety-phrase check in the same lint — an incomplete signal declaration fails the spec, while a safety phrase without a mechanism only warns.

A rejection decision (failing a build, quarantining a host, blocking a release) MUST cite each signal it relied on together with the value that signal takes in the healthy population. The Rust helper `autospec_core::spec::validate_rejection` enforces the shape of that citation: a `Rejection` whose `cited_signals` list names a signal without a `healthy_population_value` is refused, and the refusal names the missing check. A rejection that cannot say "healthy looks like X and we observed Y" has not actually checked anything.

On the shell side, `scripts/lint-signal-semantics.sh` flags pipelines that make an error indistinguishable from a zero result: a stderr discard to `/dev/null` (`2>/dev/null`, `2>>/dev/null`, optionally spaced) followed on the same line by a counting reducer (`| wc …` or `| grep -c…`). `cmd 2>/dev/null | wc -l` collapses "the command failed" and "the command found nothing" into the same `0`, and a downstream threshold check then reads the failure as healthy. Non-counting reducers (`head`, `tail`, plain `grep`, `awk`) are out of scope. The waiver is `# linter:allow-SIGNAL_SEMANTICS <reason>` on the same line or the line immediately above; the reason is mandatory and a bare marker is rejected. Default scan scope is `scripts/` and `.github/workflows/`.

## Issue Tree

AutoSpec splits a spec into a parent issue and smaller child issues. Each child issue is meant to be independently understandable and reviewable.

## Parallel decomposition

Decomposition produces an **issue DAG**: a directed acyclic graph whose edges are *hard dependencies only*. The DAG is a scheduling artifact, not a fixed work order. The executor schedules from a dynamic ready queue (an issue is ready when all of its hard predecessors have merged), and the analyzer projects **execution waves** — a diagnostic view of which issues would run together wave by wave — without replacing that dynamic scheduling.

A hard dependency MUST be added only when the dependent issue cannot be correctly implemented or independently verified against the current base branch without the predecessor (source spec section 5.1, stated verbatim).

### Valid hard-dependency reasons

Only these 8 reasons justify a hard-dependency edge. Each carries a stable `reason_code` used in the machine-readable issue metadata:

| `reason_code` | Reason (source spec section 5.1, verbatim) |
| --- | --- |
| `required-public-api` | predecessor introduces a required public API |
| `required-type-or-interface` | predecessor introduces a required type or interface |
| `required-schema` | predecessor introduces a required schema |
| `required-database-migration` | predecessor introduces a required database migration |
| `required-wire-protocol-version` | predecessor introduces a required wire/protocol version |
| `generated-artifact` | predecessor introduces a generated artifact consumed by the child |
| `structural-migration` | predecessor performs a structural migration that must precede child changes |
| `acceptance-tests-require-output` | child acceptance tests literally cannot run without predecessor output |

### Reasons that must not create an edge

The following MUST NOT create a hard dependency by themselves (source spec section 5.1):

- issue appears earlier in the spec
- issue is described as "foundational"
- implementation order would be convenient
- issues belong to the same epic
- files are nearby
- one issue is documentation
- one issue is testing
- conceptual relationship
- expected merge conflicts
- parent/child relationship
- planner preference
- "do this first" wording without technical evidence

### Conflict risk is not dependency

Issues that write the same files are tracked separately as **conflict domains** (probability, surfaces, mitigation). A high conflict score may affect dispatch ordering later, but it MUST NOT make an issue blocked: a conflict domain never blocks readiness. Conflict risk is a scheduling hint, never a dependency.

### Worked example (source spec section 31)

Bad — a chain whose edges have no justifiable reason:

```text
#1 core -> #2 API -> #3 CLI -> #4 tests -> #5 docs
```

Initial width: `1`.

Better — only the edges that carry a valid reason remain:

```text
       +-> #2 API + API tests
#1 ----+-> #3 CLI + CLI tests
       +-> #4 metrics
       +-> #5 docs
```

Initial width: `1`. After #1: `4`.

Best When Contract Already Exists — no dependency merely because all belong to the same feature:

```text
#1 core behavior + focused tests
#2 API + focused tests
#3 CLI + focused tests
#4 metrics
#5 docs
```

Initial width: `5`.

Metric definitions (initial width, maximum width, critical path, fleet saturation) live in source spec section 20 and are intentionally not duplicated here.

## Model Fit

Issues receive labels such as `ctx:*` and `reasoning:*` so operators can route work to an appropriate model or harness. The goal is not to benchmark models; it is to keep work units honest about context and reasoning needs.

## Implementation Monitor

`/autospec-run` processes ready issues, opens PRs, runs validation, asks for review, and either merges or reports blockers depending on the configured gates.

## Closeout Report

Every implementation issue should end with a result-first report: claims, proof type, before/after, artifacts, scoped git status, and the most likely hidden failure.

## Release Gate

Release readiness combines repository validation, docs drift checks, QA proof, CI state, and explicit blocker reporting. It is evidence gathering, not a marketing badge.

## Safety Boundary

AutoSpec can automate a lot of repository work, but maintainers still own production impact, credentials, destructive operations, and policy decisions.
