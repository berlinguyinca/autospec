### Added

- `name_scope` module (`crates/autospec-core/src/name_scope.rs`, issue #4251):
  name-collision detection and scope-qualified identifiers. An identifier is
  only meaningful inside the scope that issued it. Primitives:
  - `glob_matches` / `glob_literal_prefix` — a two-pointer glob matcher
    (`*` any run, `?` one char; `[` is a literal) and the literal-part
    extractor used by the prefix detector.
  - `prefix_collisions` — flags a glob whose literal part sits in one known
    identifier's name territory while reaching a different known identifier
    that continues it at a component-name boundary (the
    `qwen3.8-27b-*` vs `qwen3.8-27b-vision` case; `issue-1` vs `issue-14`
    is clean — a digit is not a boundary).
  - `ScopedId` / `parse_scoped_id` / `qualify` — the `Scope#14` form: a
    bare integer is never a cross-scope key.
  - `key_findings` — over records keyed on a number, flags unscoped records
    and numbers that appear under ≥2 scopes (the four-project
    `$LLM/*/out/issue-*` pipeline).
  - `cross_kind_collisions` — a name used for ≥2 kinds of system object
    (directory, crate, service, repository) is probably two things; the
    two-`gateway`-components case.
  - `unstated_name_uses` — a spec naming a component must state where else
    that name is used.
  - `CarriedClaim` / `ClaimVerdict` — evidence carried across instances
    must name its producer.
  - `SpecMove` / `SpecMoveVerdict` — a spec moving records between systems
    must use scope-qualified identifiers, never a bare key.
  Regression tests: `crates/autospec-core/tests/name_scope.rs`
  (19 tests, including the four incident reconstructions).
