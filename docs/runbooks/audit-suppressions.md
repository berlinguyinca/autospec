# Audit suppressions runbook

The CI `audit` job does not run bare `cargo audit`. It runs
`scripts/audit-suppressions.sh gate`, a policy layer that keeps the audit gate
live for new advisories while recording reviewed, time-boxed exceptions for
advisories with no available fix.

## When a suppression is justified

A suppression is an admission that a known-vulnerable crate is linked into the
build. It is only acceptable when **all** of the following hold:

1. `cargo audit` reports no fixed upgrade for the advisory, and
2. the vulnerable code path is unreachable in this workspace (state the
   reachability argument in `reason`), and
3. a concrete, checkable removal trigger exists.

Any suppression that cannot state a `reason` and a `removal_trigger` is
rejected by the tooling (fail-closed) — an unexplained ignore is a policy
error, not a pass.

## Entry contract

One file per advisory under `config/audit-suppressions/`, named
`RUSTSEC-YYYY-NNNN.txt` (filename must equal the `advisory` field):

```
advisory: RUSTSEC-2023-0071
since: 2026-09-10
dependency_path: <vulnerable crate version <- ... <- workspace crate>, consumed at <path>
reason: <why this is safe today, including the reachability argument>
removal_trigger: <concrete condition under which this file is deleted>
```

All five keys are required. `since` must be a real calendar date, not in the
future. First occurrence of a key wins; extra keys are ignored.

## Commands

```bash
# Validate the metadata only (no cargo-audit needed). Exit 0 ok, 2 invalid.
bash scripts/audit-suppressions.sh validate

# Full gate: validate, then run cargo-audit; pass only if every failing
# advisory has a valid suppression. Exit 0 ok, 1 advisory failure,
# 2 invalid entry, 3 missing cargo-audit/cargo.
bash scripts/audit-suppressions.sh gate

# Report every active suppression with its age in days. Exit 0 ok, 2 invalid.
# Override the clock (e.g. for tests or backdated entries):
bash scripts/audit-suppressions.sh report --today 2026-09-10
```

All subcommands accept `--dir PATH` to point at a non-default suppression
directory.

## Adding a suppression

1. Let `cargo audit` fail and read the advisory.
2. Confirm no fixed upgrade exists and write the reachability argument.
3. Add `config/audit-suppressions/RUSTSEC-YYYY-NNNN.txt` with all five keys.
4. Run `bash scripts/audit-suppressions.sh gate` locally; the gate prints
   `AUDIT_SUPPRESSED:<id>` with the recorded reason and removal trigger.

## Removing a suppression

Delete the file when its `removal_trigger` becomes true (upstream fix lands in
`Cargo.lock`, the dependency or its vulnerable feature is dropped, or the
advisory is withdrawn). The `report` step in CI prints each suppression's age
on every run so stale entries are visible.

## Tests

`tests/audit-suppressions.bats` covers the contract: a suppressed advisory
passes the gate, an unsuppressed one fails it, and entries missing `reason` or
`removal_trigger` are rejected. The suite stubs `cargo-audit` with a fake
binary on `PATH` and needs no network access.
