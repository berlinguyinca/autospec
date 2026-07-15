# Ready Queue Draft Admission Design

## Goal

Ensure the Rust ready queue treats classification drafts as blocked until they
carry `auto-implement` and no longer carry `needs-classify`.

## Decision

Admission happens before safety, dependency, pull-request, and path-conflict
work. A candidate with `needs-classify` receives the stable blocked reason
`needs_classify`; a candidate without `auto-implement` receives
`missing_auto_implement`. Neither can appear in `ready` or `batch`.

This is a Rust-only policy repair. Classification remains responsible for the
label transition; the queue only consumes the resulting state.

## Tests

Core tests cover a draft, an unlabeled issue, and a promoted issue. CLI JSON
coverage verifies the blocked reason is visible to operators. Full workspace
tests prove the policy does not affect existing queue ordering.
