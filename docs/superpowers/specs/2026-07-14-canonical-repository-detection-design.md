# Canonical repository detection design

**Issue:** #1483
**Status:** approved by the operator's standing "do all" instruction
**Decision:** add a typed Rust command; do not revive the retired shell org-sweep path.

## Problem

An organization sweep can observe the same defect in a current umbrella repository and in an
archived split repository, mirror, fork, or generated downstream repository. Filing each
observation separately creates duplicate work and can send a repair to the wrong repository.
The historical implementation surface was shell-oriented and has been retired, so restoring it
would violate the Rust control-plane cutover.

## Command and input

`autospec explore repositories --input <path>` accepts one JSON document:

```json
{
  "repositories": [{
    "name": "metabolomics-us/go-modules",
    "family": "go",
    "archived": false,
    "revival_requested": false,
    "pushed_at": "2026-07-14T00:00:00Z",
    "readme": "active umbrella for Go modules",
    "module_paths": ["example.com/go-modules"],
    "packages": ["admin"],
    "dependency_references": ["metabolomics-us/go-admin"]
  }],
  "findings": [{"repository":"metabolomics-us/go-admin","fingerprint":"lint-x","title":"Fix lint"}]
}
```

Every repository belongs to one `family`; comparisons never cross family boundaries. The command
uses only checked-in input and emits deterministic JSON, so it has no network side effects.
`revival_requested` is valid only for an archived repository. `module_paths` and `packages` must
contain non-blank, unique entries so repeated evidence cannot inflate a score.

## Inference and routing

For each family, the Rust core produces a score from all evidence fields: active/archived state,
push recency rank, non-empty README, module-path count, package count, and inbound dependency
references. Archived repositories have a large negative score and are listed as
`do_not_file_by_default`, except when `revival_requested` is true. The highest eligible score
(then repository name as a stable tie-breaker) becomes the canonical target.

The report also exposes `repository_scores` for every input repository, sorted by family then
repository name. This audit list retains the archive penalty for non-revival archives even though
they are excluded from canonical-target selection. A requested revival is an archived exception:
it is eligible, receives revival treatment, and does not inherit the archive penalty.

Findings route to their repository family's canonical target. The report retains the first
finding for each `(canonical_target, fingerprint)` pair and marks later copies as duplicates.
If a family has no eligible target, its findings are deferred rather than filed against an
archived repository.

## Integration boundary

The core model and JSON parser/renderer live under `crates/autospec-core/src/exploration/`; the
CLI adapter lives under `crates/autospec-cli/src/commands/explore.rs`. The three
`autospec-explore` prompt bodies add an identical contract directing org sweeps to invoke the
Rust command and consume `routed_findings`. They do not add a shell or Python implementation.
The rendered JSON includes `canonical_targets`, `repository_scores`,
`do_not_file_by_default`, `routed_findings`, and `deferred_findings` in a stable field order.

## Validation

Rust tests cover score inputs, archived/revival behavior, contradictory revival rejection,
non-inflating module/package evidence, canonical tie-breaking, duplicate routing, audit scores,
and CLI JSON rendering. The existing Rust structural validator proves the three prompt bodies
remain lock-step; `autospec validate --fast` is the repository-level check.
