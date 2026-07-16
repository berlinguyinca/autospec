# Rust Autonomous Repository Configuration Design

**Issue:** #1602
**Parent:** #2076 / #1861
**Status:** approved by the standing Rust-cutover execution directive

## Goal

Make Rust mainline-health admission read repository-local
`.autospec/autonomous.yml`, so a repository can choose its protected branch
and advisory checks without global shell environment policy.

## Scope

This child supports only:

```yaml
main_health:
  branch: master_ai
  ignore_checks:
    - Unit Tests
```

The precedence is `--branch` > `main_health.branch` > GitHub default branch.
`ignore_checks` matches exact check names. Matching evidence remains recorded
but becomes advisory, so it cannot block mainline-health admission.

## Architecture

`autospec_core::autonomous::config` owns a dependency-free, no-I/O parser and
typed `AutonomousConfig` / `MainHealthConfig` values. It ignores unrelated
top-level policy keys but rejects malformed, duplicate, wrongly typed, or
unknown fields within `main_health`.

The CLI reads `<checkout-root>/.autospec/autonomous.yml` once per invocation
and passes the typed branch and advisory set directly to the existing Rust
health model. A `--repo-dir` inside a Git checkout resolves to its checkout
root; a non-Git directory is used literally. A missing file retains current
defaults. Any relevant config read or parse failure is a diagnostic before
queue selection, claim mutation, or executor dispatch.

## Rejected alternatives

1. Call the existing shell policy resolver. Rejected: this preserves shell
   authority and blocks #2076 deletion.
2. Export `AUTOSPEC_MAIN_HEALTH_*`. Rejected: environment propagation leaks
   policy across repositories and recreates this issue's root cause.
3. Add a generic YAML dependency. Rejected: the bounded schema needs no new
   dependency; this parser has no generic YAML ambition.
4. Preserve regex ignores. Rejected: exact names are deterministic and do not
   reintroduce the shell's global-regex authority.

## Data contract

```rust
pub struct AutonomousConfig {
    pub main_health: MainHealthConfig,
}

pub struct MainHealthConfig {
    pub branch: Option<String>,
    pub ignore_checks: BTreeSet<String>,
}
```

The parser accepts comments and quoted/unquoted nonempty scalar/list values.
It rejects duplicate `branch` or `ignore_checks`, empty values, scalar
`ignore_checks`, inline/nested collections, and malformed indentation in the
relevant block. `HealthBranchInput` gains `configured_branch`, with a new
`Configured` source. A pure helper converts matching required evidence to
advisory without removing or changing the evidence itself.

## Boundaries

- Do not modify `scripts/autonomous-resilience.sh`, autonomous launch scripts,
  shell waterfall code, or skill trios.
- Rust must not read/export `AUTOSPEC_MAIN_HEALTH_*`.
- Do not add drain, no-work, premerge, retry, or other future config keys.
- Advisory mainline checks do not relax premerge, safety, claims, or merging.

## Tests and completion

Core tests cover parser cases, branch precedence, advisory evidence, and
unmatched blocking checks. CLI fixtures prove branch selection, CLI override,
per-repository isolation, malformed-config pre-dispatch failure, and absence
of shell/environment authority. Full Rust workspace and fast validation must
pass before the #2076 deletion child consumes this config path.
