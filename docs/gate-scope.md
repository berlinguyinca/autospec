# Gate scope

A gate set is scoped to a language. A converted patch once passed the whole
set — format clean, clippy clean, 678 tests green — while every file it
changed was TypeScript: every number true, none of them evidence about the
change (#3793, #3928). Gate scope makes that state unrepresentable.

## The mapping

`config/gate-scope.yml` maps file types to the gates that read them:

- `version: 1`
- `scopes:` — one entry per file type, in **priority order** (the first
  entry whose criteria all match a path claims it):
  - `type` — the file-type label (unique)
  - `match` — `extensions` (without dots, case-insensitive) and/or a
    `path_prefix`; criteria are ANDed, and an entry with no criteria is a
    parse error (a typo, not a wildcard)
  - `gates` — the gates that read that file type (non-empty)

The shipped entries: Rust (`.rs` → the cargo gates), web
(`apps/web/` → the npm gates), shell script (`.sh`, `.bash` → `bash -n`),
bats (`.bats` → `bats`), workflow (`.github/` + `.yml`/`.yaml` → workflow
parse).

## Behavior

Implemented as pure primitives in
[`crates/autospec-core/src/execution/gate_scope.rs`](../crates/autospec-core/src/execution/gate_scope.rs):

1. The gate set for a patch is the union of the gates of the file types it
   touches — derived from the mapping, never from memory
   (`GateScopeMapping::gate_set_for`).
2. A changed path whose type no **executed** gate reads is reported as
   `UNVERIFIED: <path> is not read by any gate in this run` — a refusal,
   never a pass, however green the gates that did run were
   (`verdict`).
3. A gate result records the file types it reads, and a verdict rendered
   beside a diff names the file types it covers: `VERIFIED (covers: Rust,
   shell script)`, with each result line carrying `(scope: …)`
   (`GateResult::line`, `Verdict::line`).
4. A path no entry claims, and a mapping with no entries, fail closed:
   unverified, not passed.

## Extending

Adding a file type is one entry in `config/gate-scope.yml` plus one test
case (the parser is strict: unknown keys, empty gate lists, missing
criteria, and duplicate type labels are all errors, so a broken mapping
fails loudly rather than silently claiming less than it says it claims).
A change that touches only an unmapped type is reported unverified until
the mapping is extended — which is the point.
