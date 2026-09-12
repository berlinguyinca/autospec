# Name scope and collision (issue #4251)

An identifier is only meaningful inside the scope that issued it. Four
collisions made that concrete: a worker glob `qwen3.8-27b-*` that also
matched `qwen3.8-27b-vision-*`; a pipeline collecting `$LLM/*/out/issue-*`
across four projects where `issue-1` meant four different things; two
instances (edge and hive) both called `gateway`; and two components named
`gateway` in two repositories — one a directory, one a crate, one a service,
one a repository.

- **A glob collides when its literal part sits in one known identifier's
  name territory while reaching a different known identifier that continues
  it at a component-name boundary.** The boundary is the separator set
  (`NAME_SEPARATORS`: `-`, `_`, `/`, `.`), not a digit: `issue-1` vs
  `issue-14` is clean, `qwen3.8-27b` vs `qwen3.8-27b-vision` is not
  (`prefix_collisions`, parameterized separators).
- **A bare integer is never a cross-scope key.** Records keyed on a number
  flag unscoped records and numbers that appear under ≥2 distinct scopes —
  the four-project `issue-*` pipeline (`key_findings`). The qualified form
  is `Scope#14` (`ScopedId`, `parse_scoped_id` rejects a bare number, `qualify`).
- **A name used for ≥2 kinds of system object (directory, crate, service,
  repository) is probably two things** (`cross_kind_collisions`; two kinds
  is the sensitivity — one kind is just a name).
- **Evidence carried across instances must name its producer**
  (`CarriedClaim::verdict` — `Unattributed` is a finding, `Attributed` is
  clean; `Local` for in-instance use).
- **Specs encode scope explicitly.** A spec naming a component states where
  else that name is used (`unstated_name_uses`); a spec moving records
  between systems uses scope-qualified identifiers, never a bare key
  (`SpecMove::verdict` — `BareKey` over ≥2 systems is a finding).

Checkable in `autospec_core::name_scope` (`glob_matches`,
`prefix_collisions`, `ScopedId`, `key_findings`, `cross_kind_collisions`,
`unstated_name_uses`, `CarriedClaim`, `SpecMove`). Tests:
`crates/autospec-core/tests/name_scope.rs`.
