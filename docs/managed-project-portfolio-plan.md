# Portfolio plan manifest (`autospec.portfolio-plan.v1`)

A portfolio plan is the frozen description of everything a managed project intends to
build: the repositories it spans, the capability observed for each, the items to
materialize, and the edges between them. Freezing is a pure act — reading the plan,
validating it, and dry-running it performs no durable or remote write.

Rust surface: `commands::managed_project::portfolio` in the `autospec` crate.

## Canonical form

One document, deterministic key order (alphabetical within each object), two-space
indentation, every scalar double-quoted except YAML's own `null` and `[]`:

```yaml
items:
  - completion_policy: "portfolio-gate"
    depends_on: []
    item_key: "tracker/source"
    local_parents: []
    repository: "acme/spec"
    role: "source-tracker"
  - completion_policy: "self-closing"
    depends_on:
      - "tracker/source"
    item_key: "alpha:build"
    local_parents: []
    repository: "acme/alpha"
    role: "implementation"
plan_digest: "<sha256>"
portfolio_id: "<sha256 of the source spec identity>"
primary_scope: "product:acme"
project_owner: "acme"
repositories:
  - capability: "available"
    name: "acme/alpha"
    observed_revision: "2222222222222222222222222222222222222222"
  - capability: "available"
    name: "acme/spec"
    observed_revision: "1111111111111111111111111111111111111111"
schema: "autospec.portfolio-plan.v1"
source_spec: "acme/spec:docs/specs/portfolio.md@a3a3...a3"
```

`primary_scope` is `null` until the plan declares one; the selector is part of the frozen
document and therefore part of the digest.

## Digest

```
sha256( "autospec.portfolio-plan.digest.v1" ++ "\n" ++ canonical_yaml_without_digest )
```

The digest is computed over the canonical rendering, so two drafts carrying the same
facts in different input order freeze to the same value. An `Option` field renders as
YAML `null` and a required field as an empty string, which keeps a tampered document from
reproducing a digest by deleting a key.

A frozen plan is re-verified with `PortfolioPlan::validate`, which recomputes the digest.
Any field edited after the freeze — a revision, a capability, an edge, an added item —
fails verification instead of being silently accepted.

## Capability facts

Each repository carries exactly one of three states, recorded at read time:

| State | Meaning | Effect on freeze |
|---|---|---|
| `available` | reachable, revision read | item may be planned |
| `unavailable` | read attempted, repository refused or missing | refusal |
| `unknown` | never probed | refusal |

An unprobed repository is never planned over: the plan must state that nobody looked,
which is a different fact from somebody looking and being refused, so the two get
different codes and different exit values.

## Rejections and exit codes

The Rust test `commands::managed_project::portfolio::tests::every_documented_exit_code_is_distinct`
parses the table below and refuses a code that is missing from it, duplicated, listed with
the wrong exit value, or missing from the code table.

| Code | Exit | Meaning |
|---|---|---|
| `SCHEMA_UNSUPPORTED` | 20 | manifest names a schema other than `autospec.portfolio-plan.v1` |
| `OWNER_MISSING` | 21 | no project owner declared |
| `OWNER_INVALID` | 22 | owner is not a safe GitHub owner name |
| `PORTFOLIO_SET_EMPTY` | 23 | manifest declares no repositories |
| `REPOSITORY_INVALID` | 24 | repository name is not `owner/name` |
| `REPOSITORY_DUPLICATE` | 25 | same repository declared twice |
| `REPOSITORY_CAPABILITY_UNKNOWN` | 26 | item hosted by an unprobed repository |
| `REPOSITORY_CAPABILITY_UNAVAILABLE` | 27 | item hosted by an unreachable repository |
| `ITEM_KEY_INVALID` | 28 | item key rejected by `ItemKey` grammar |
| `ITEM_KEY_DUPLICATE` | 29 | two items share one key |
| `ITEM_REPOSITORY_UNDECLARED` | 30 | item hosted outside the declared portfolio |
| `EDGE_DUPLICATE` | 31 | same edge declared twice |
| `EDGE_SELF_DEPENDENCY` | 32 | item depends on itself |
| `EDGE_REFERENCE_MISSING` | 33 | edge points at no item in the plan |
| `LOCAL_PARENT_CROSS_REPOSITORY` | 34 | `local_parents` entry crosses repositories |
| `DEPENDENCY_CYCLE` | 35 | dependency cycle detected |
| `DIGEST_MISMATCH` | 36 | frozen digest does not match its content |
| `PRIMARY_SCOPE_UNDECLARED` | 40 | no scope and no single host to derive it from |
| `PRIMARY_SCOPE_AMBIGUOUS` | 41 | several hosts and no declared scope |
| `PRIMARY_SCOPE_UNKNOWN` | 42 | declared scope names no host in the plan |

Three of these are the DAG rejections: a cycle (35), a self-dependency (32), and an edge
referencing an item that is not in the plan (33). A cross-repository `local_parents`
entry (34) is the fourth, since that field is the one edge type that must never cross a
repository boundary. Cross-repository dependencies are expressed only through
`depends_on`.

## Dry-run and materialization

The dry run is the only path to a materialization permit, and it reports:

- `mutations().durable()` — writes to the journal, store, or git. Always `0`.
- `mutations().remote()` — calls to GitHub or package registries. Always `0`.
- `verify_zero_mutations()` — `Err` if either counter is non-zero.
- `checked_paths()` — filesystem entries witnessed before and after the run, proving the
  walk itself changed nothing.
- `execution_order()`, `item_count()`, `repository_count()`, `primary_scope()`.

`MaterializationPermit::grant` is the only way to obtain a permit, and it refuses a
report that is not clean. The permit grants permission; it does not claim work happened.
Nothing in this module writes.
