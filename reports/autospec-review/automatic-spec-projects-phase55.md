# Phase 5.5 audit + remediation — automatic spec Projects (issue #3444)

Audit of the managed-Project recovery, marker-integrity, lease, blocker, and
completion-gate behaviour that ships in this revision of the autospec CLI,
plus the status of the deployment conformance receipt (#3443) this audit gates.

Evidence kinds used below:

- **bats** — `tests/portfolio-phase55.bats` (14 scenarios, recorded-fixture
  `gh` replacer, real CLI binary, durable state under a scratch
  `AUTOSPEC_HOME`; runs in default CI, hermetic).
- **rust** — `crates/autospec-cli/tests/managed_project.rs` (112 integration
  tests, recorded protocol fixtures, `AUTOSPEC_GH_PROGRAM` injection) and the
  `#[cfg(test)]` units in `crates/autospec-cli/src/commands/managed_project/`.
- **static** — grep/source inspection of this revision (recorded as static
  evidence; consumers must not treat it as runtime proof).

## 1. Design test-group coverage (16/16 mapped)

| # | Design test group (spec §Testing) | Status in this revision | Evidence |
|---|-----------------------------------|-------------------------|----------|
| 1 | Stable identity, source blob changes, owner selection, marker pagination, one-match adoption, duplicate ambiguity, lost-create-response recovery | **Covered** for product identity and marker paths | rust: `managed_project_portfolio_identity_is_stable_and_changes_with_source_blob`, `managed_project_portfolio_identity_preserves_component_boundaries`, `github_resolve_adopts_one_exact_marker_and_ignores_title_only_matches`, `github_resolve_rejects_ambiguous_markers_without_mutation`, `github_resolve_fails_closed_when_project_discovery_may_be_truncated`, `github_pending_create_without_identity_never_creates_a_second_project`; bats: "repeated resolve adopts the same marked Project", "two Projects bearing the exact marker", "lost create response fails closed" |
| 2 | Private manifest permissions, atomic writes, partial-tail recovery, unsafe path rejection, reconstruction, outbox replay | **Covered** | rust: `store_read_only_rejects_a_symlinked_or_public_ancestor_before_reading_state`, `store_read_only_replays_newest_valid_events_without_repairing_files`, `store_read_only_rejects_nonempty_binding_without_valid_journal`, `store_duplicate_event_keys_are_no_ops`, `store_reopens_repository_edge_and_pending_projection_from_journal`, `store_global_open_imports_one_legacy_repo_state_and_reuses_it_across_repositories`; bats: "policy owner drift conflicts with the durable binding before any remote call" |
| 3 | Projects v2 creation, exact field types/options, table/board views, incompatible fields, human-text preservation, escaped Mermaid | **Partially covered** | Creation, exact readme-string round-trips, and human-text preservation are covered (rust: `github_resolve_creates_marks_verifies_and_persists_when_no_marker_matches`, `github_legacy_product_adoption_migrates_the_existing_marker_block`, unit `marker_schema_migrates_legacy_product_without_losing_human_text`). Field-type/options and view-mutation surfaces are not exercised by any shipped command because this revision implements no field/view mutation CLI; recorded as a finding, not a defect of shipped code (no surface exists to misuse). |
| 4 | Cross-repository planning, capability preflight, one local tracker, canonical URL dependencies, partial filing resume, no duplicate items | **Not implemented** | No planning CLI exists in this revision. Underlying canonical-URL item identity and duplicate-item rejection are covered at the reconcile layer (rust: `github_item_reconciliation_ignores_known_nonissues_but_rejects_unknown_items`, `github_issue_urls_require_nonempty_identity_and_positive_canonical_number`, `github_reconcile_is_idempotent_and_journals_failures_before_item_add`); the planning/tracker layer above them is a finding. |
| 5 | Status transitions for queued/claimed/PR-open/review/blocked/failed/merged/reopened/post-merge/audit | **Not implemented** | No status-field writer exists in the CLI (static: no status-mutation call site in `crates/autospec-cli/src/commands/managed_project/`). Item membership + pending-projection retry is the shipped projection surface (bats: "sync reconciles item membership through the journaled projection"). Finding. |
| 6 | Completion proof failing on any outstanding child, prerequisite, tracker, or Phase 5.5 audit | **Covered by construction** | There is no completion writer at all: `managed_state done` is never written and no CLI surface emits a done state (bats: "no CLI surface claims completion: managed_state done is never written"; static: no completion writer call site in the CLI). A "no verified completion" claim is therefore impossible from this revision, which is the safe direction. Finding: positive completion proof (fail-fast on outstanding children) still requires the status surface of group 5. |
| 7 | Issue-definition entry points provision the portfolio before first `gh issue create`; lock-step skill bodies | **Not implemented** | No issue-entry-point integration in this revision. Finding; blocked on groups 4/5. |
| 8 | `--dry-run` proves zero remote/durable mutation, planned shape, tri-state capabilities | **Covered** | rust: `onboard_cli_dry_run_emits_stable_sorted_json` (asserts zero state mutation and stable sorted output); bats: every failing scenario in `tests/portfolio-phase55.bats` additionally asserts exact mutation call counts from the recorded `gh` invocation log. |
| 9 | Deterministic failpoints: lost create responses, two-host lease races, pagination, mid-field failure, item-add ambiguity, rate-limit replay, journal-tail recovery, credential revocation | **Partially covered** | Lost product-create response (bats "lost create response fails closed"; rust `github_create_failure_persists_identity_and_resumes_marker_edit_without_duplicate_create`), lost/ambiguous marker write (bats "ambiguous marker write is journaled and resumes to the verified Project"; rust `github_ambiguous_marker_edit_resumes_from_verified_bound_project`, `onboard_cli_journals_selected_issue_before_relationship_fetch_failure`), pagination truncation (rust `github_resolve_fails_closed_when_project_discovery_may_be_truncated`), journal-tail replay (rust `store_read_only_replays_newest_valid_events_without_repairing_files`), credential revocation on reads (rust `gh_cli_read_auth_403_is_a_hard_nonzero_onboarding_failure`, `gh_cli_transient_read_failure_keeps_the_typed_pending_outcome`). Two-host lease races and rate-limit replay: see findings §3. |
| 10 | Opt-in real-GitHub smoke tests; default CI hermetic with recorded fixtures | **Covered for the shipped surface** | Default CI is hermetic: all bats/rust evidence above uses recorded protocol fixtures via `AUTOSPEC_GH_PROGRAM`. The opt-in live smoke runner is not in this revision; finding (no live surface to smoke yet beyond product Project resolve/sync, which a future opt-in run can cover). |
| 11 | Permission tests: missing `project` scope, owner denial, inference denial, private cross-owner visibility, inaccessible repos, rate limits, mid-run revocation | **Partially covered** | Definitive 403 on Project listing blocks with a diagnostic and no mutation (bats: "definitive 403 on Project listing blocks with a diagnostic and no mutation"; rust `gh_cli_owner_enumeration_auth_failure_is_hard_before_onboarding_state`), owner-deny before enumeration (rust `onboard_cli_rejects_an_invalid_issue_before_owner_enumeration`, `onboard_cli_requires_an_allowlist_for_owner_enumeration`). Rate-limit retry-after replay exists in the transport classification (`GithubFailure::RetryAfter`); a dedicated failpoint test is a finding. |
| 12 | Router tests: installed-compatible Autospec receives implementation intent, direct serverless dispatch unreachable | **Not implemented** | Router/handoff is out of scope for this revision's CLI. Finding; this is the deployment-side gate for the #3443 receipt. |
| 13 | Scope-routing: spec lineage, spec-sized vs bounded work, ambiguity blocks, primary+secondary projection, key round trips, collision resistance, migration, Windows-safe paths | **Partially covered** | Key round trips and collision resistance (rust: `managed_project_namespaces_are_collision_safe_and_round_trip`, `managed_project_item_key_serialization_has_a_durable_golden`, `managed_project_identity_types_reject_unsafe_keys`), legacy migration (rust: `managed_project_binding_evolves_legacy_products_to_schema_two`, `github_legacy_product_migration_requires_the_schema_two_marker_on_requery`), ambiguity blocks rather than guesses (bats + rust, group 1). Spec-lineage routing and secondary boards are findings. |
| 14 | Lease preflight: ref read/create/fast-forward, branch/ruleset denial, two-host races, mid-run revocation | **Not implemented** | No lease surface ships in this revision: `autospec project lease` is refused as an unknown subcommand and `autospec portfolio …` as an unknown command (bats: "lease and portfolio-transaction surfaces are refused, not faked"). Because no lease surface exists, there is no lease to race — and no "no verified transaction identity" claim to make either. See finding F-3. |
| 15 | Marker tests: legacy product migration, spec-portfolio adoption inside existing block, duplicate/mixed markers, kind mismatch, no second parser grammar | **Covered** | Legacy migration (rust: `marker_schema_migrates_legacy_product_without_losing_human_text`, `github_legacy_product_adoption_migrates_the_existing_marker_block`), duplicate/mixed markers fail closed (bats: "duplicate managed marker blocks fail closed"; unit `github_marker_parser_requires_one_complete_exact_marker`), kind/owner mismatch (rust: `github_bound_spec_portfolio_rejects_product_kind_without_mutation`, `github_resolve_fails_closed_on_marker_owner_mismatch`; bats: "marker owned by another organization is a hard identity conflict", "bound Project presenting a different marker fails closed without mutation"), single parser grammar (static: one `parse_marker` in `github/parse.rs`, no second grammar). |
| 16 | External-consumer conformance tests reject a bad receipt | **Blocked** | The conformance receipt tooling and the direct-dispatch router it validates are not in this revision; see §4 (receipt #3443 = NOT-VERIFIABLE). |

## 2. Adversarial areas (spec §5 remediation list)

### Area 1 — lost responses

- **Lost `create` response, product identity.** The create intent is journaled
  *before* the mutation (`create_projection` pending before `gh project
  create`), and a later run with no verified identity fails closed with
  `pending project creation has no verified project identity` — it never
  blindly re-creates (bats: "lost create response fails closed and never
  blindly re-creates"; rust:
  `github_pending_create_without_identity_never_creates_a_second_project`,
  `github_create_failure_persists_identity_and_resumes_marker_edit_without_duplicate_create`).
- **Lost `create` response with verified identity.** Spec-portfolio creation
  journals a recovery capsule whose `create_nonce` is embedded in the title
  (`{title} [autospec:{nonce}]`); a rerun recovers exactly one candidate
  bearing that exact nonce title and refuses two. Rust evidence:
  `autonomous_accountability_github_spec_portfolio_create_unknown_recovers_one_nonce_title_candidate`,
  `github_spec_portfolio_create_unknown_waits_when_nonce_title_is_not_visible`,
  `github_spec_portfolio_create_unknown_refuses_two_nonce_title_candidates`,
  `github_spec_portfolio_rejects_untrusted_or_oversize_capsules_before_mutation`.
  The CLI's product path does not yet construct spec-portfolio identities;
  the capsule mechanism is shipped and tested at the library surface.
- **Lost `edit` (marker write) response.** The marker edit is journaled as a
  pending projection; the failure surfaces as
  `cannot write managed GitHub Project marker: <gh error>`, and a rerun
  resumes from the verified bound Project, re-reads the README, acks the
  projection on an exact marker match, and persists — with exactly one edit
  across both runs (bats: "ambiguous marker write is journaled and resumes
  to the verified Project"; rust: `github_ambiguous_marker_edit_resumes_from_verified_bound_project`).
- **Definitive failures** (e.g. 403 missing `project` scope) are classified
  separately from ambiguous transport failures and block with
  `cannot create managed GitHub Project: <err>` / `cannot list GitHub
  Projects: <err>` without any mutation (bats: "definitive 403 on Project
  listing blocks with a diagnostic and no mutation"; rust:
  `gh_cli_read_auth_403_is_a_hard_nonzero_onboarding_failure`).

### Area 2 — duplicate and invalid markers

- **Multiple Projects bearing the exact marker** for one product block
  resolution with `multiple GitHub Projects have the managed marker for
  <product_key>`; no local state is written (bats: "two Projects bearing the
  exact marker are ambiguous and block resolution"; rust:
  `github_resolve_rejects_ambiguous_markers_without_mutation`,
  `autonomous_accountability_github_spec_portfolio_adopts_exactly_one_marker_bearing_project`).
- **Multiple or malformed marker blocks** in one README fail closed before
  any mutation: the marker diagnostic is
  `GitHub Project managed marker must contain exactly one complete block`
  (bats: "duplicate managed marker blocks fail closed"; unit:
  `github_marker_parser_requires_one_complete_exact_marker`).
- **Marker owned by another organization** is a hard identity conflict, not
  a migration and not an overwrite: the diagnostic reads `managed GitHub
  Project marker owner otherorg conflicts with approved owner phase55org`
  (bats: "marker owned by another organization is a hard identity conflict";
  rust: `github_resolve_fails_closed_on_marker_owner_mismatch`).
- **Bound Project later presenting a different marker** (identity swapped
  under us) fails closed with `GitHub Project contains a different managed
  marker` and zero remote mutations after the bind (bats: "bound Project
  presenting a different marker fails closed without mutation").
- **Durable binding drift** (policy owner changed after a bind) conflicts
  before any remote call: `managed project binding owner conflicts with
  policy` (bats: "policy owner drift conflicts with the durable binding
  before any remote call").
- **Legacy markers** are adopted and migrated in place to the schema-2 block
  without losing human README text (rust evidence in §1 rows 1/15).

### Area 3 — lease races

**Finding F-3 (gap, by design of this revision).** No lease or
portfolio-transaction surface ships: `autospec project lease` →
`unknown autospec project subcommand: lease`, `autospec portfolio …` →
`unknown autospec command: portfolio` (bats: "lease and portfolio-transaction
surfaces are refused, not faked"). Consequences:

1. Concurrent autospec processes are **not** mutually excluded. Durability is
   single-process: the journal is an append-only high-watermark log replayed
   read-only, duplicate event keys are no-ops, and a corrupt/torn tail drops
   only the incomplete newest line (rust: `store_duplicate_event_keys_are_no_ops`,
   `store_read_only_replays_newest_valid_events_without_repairing_files`).
   Two hosts mutating the same `AUTOSPEC_HOME` state directory simultaneously
   can interleave writes; the store is fail-closed on the next open — a
   binding without a valid journal is refused and replay is read-only — but
   it does not arbitrate concurrent writers.
2. Because no transaction identity exists, the audit cannot record a "no
   verified transaction identity" diagnostic — there is nothing to verify
   one against. The safe behaviour is in place (refuse unknown surfaces
   rather than fake them); the race-safety machinery (ref read/create/
   fast-forward preflight, two-host failpoint) is deferred with the router
   (design group 14) and must land before any multi-host automation runs
   against one state root.

### Area 4 — blockers

**Finding F-4 (gap, by design).** No blocker classification or
blocker-field writer ships, so "blocked" cannot be *projected* yet — but it
also cannot be *silently dropped*: nothing in this revision can mark work
done, so an outstanding prerequisite or blocker can never be masked as
complete. The shipped enforcement point is the completion gate (Area 5) and
the queue-side behaviour of existing autonomous accountability (unchanged).
Blocker projection is deferred with the status surface (design group 5).

### Area 5 — completion gates

- **No completion is claimable.** No command in this revision writes a
  `managed_state done` record or emits a done state: the state directory
  contains only `binding.json` + `events.jsonl` with schema-versioned
  projection state (bats: "no CLI surface claims completion: managed_state
  done is never written"; static: no completion writer in
  `crates/autospec-cli/src/commands/managed_project/`).
- **Repository-local completion stays Blocked** until the
  deployment-owned consumer publishes a matching
  `autospec.implementation-handoff.v1` conformance receipt (spec
  §Acceptance criteria). In this revision that receipt cannot yet be
  verified — see §4 — so the Blocked state is the correct, safe status.

## 3. Findings register

| ID | Severity | Finding | Disposition |
|----|----------|---------|-------------|
| F-1 | info | Status-field writers, views/fields mutation, planning/tracker provisioning, spec-lineage routing, and the issue-entry-point integration (design groups 3-partial, 4, 5, 7, 12, 13-partial) are not implemented in this revision. The shipped surface (identity, journal, marker, create/adopt, item reconcile, sync) is fail-closed at every boundary the tests probe. | Deferred by scope; each is an independent future issue. None weakens the fail-closed posture of shipped code. |
| F-2 | info | Rate-limit `RetryAfter` classification exists in the transport but has no dedicated deterministic failpoint test (design group 11). | Future failpoint issue. |
| F-3 | warning | No lease/transaction surface ⇒ no multi-host arbitration of one state root; divergence is detected, not prevented (§2 Area 3). | Blocking only for multi-host automation; document in runbook; land with the router (group 14). |
| F-4 | info | No blocker projection surface; completion cannot be claimed in any direction until the status surface lands (§2 Area 4). | Deferred with group 5. |
| F-5 | blocking | Deployment conformance receipt #3443 is NOT-VERIFIABLE in this environment (§4). Repository-local completion therefore remains **Blocked** per spec §External consumer. | Requires a deployment-owned verification environment with authenticated GitHub access and the receipt tooling. |

## 4. Conformance receipt #3443 status

**Status: NOT-VERIFIABLE** (evidence gap, not a verdict of non-conformance).

Basis, all checked in this environment:

1. **No GitHub authentication.** `gh auth status` reports no authenticated
   account in this checkout's execution environment, so no live
   `projectsV2` read, `ref`/branch preflight, or router probe against a
   deployment-owned consumer can be executed.
2. **Receipt tooling absent from this revision.** The conformance receipt
   validator and the `autospec.implementation-handoff.v1` router it audits
   (design groups 12, 16) are not part of this revision's CLI — there is no
   surface in the shipped binary to bind a receipt to a deployed revision.
3. **Consequence.** Per spec §External consumer, repository-local completion
   stays **Blocked** until a deployment-owned verification run produces a
   matching receipt; this audit must not downgrade that status on static
   evidence alone. A "no verified completion" claim is explicitly refused
   here.

What a future verification run must record to flip the status:

- receipt schema `autospec.implementation-handoff.v1` with a
  revision digest equal to the deployed binary,
- a router probe proving direct serverless implementation dispatch is
  unreachable while an installed compatible Autospec is present,
- typed status/cancellation responses from the consumer,
- executed against authenticated GitHub with the fixture owner/repositories
  declared in the receipt.

## 5. Recovery runbook

Operator-facing recovery guidance for every adversarial scenario above
(exact diagnostics, journal inspection, safe reset, and the
explicitly-forbidden manual fixes) is in
[`docs/runbooks/portfolio-recovery.md`](../../docs/runbooks/portfolio-recovery.md),
updated in this issue to match the implemented behaviour of this revision.

## 6. Verification evidence for this audit

- `bash tests/portfolio-phase55.bats` — 14/14 pass (recorded fixtures, real
  binary, mutation-call accounting).
- `cargo test --workspace --no-fail-fast` — all managed-Project tests green
  (the 112 in `tests/managed_project.rs` plus store/parse units). This
  environment carries 16 pre-existing, unrelated failures: the
  `executor_bridge` unit tests require `codex`/`gitleaks`/`semgrep` on PATH
  (absent here) and the `issue_commands` suite hits a `jq` 1.6 parsing gap in
  this shell. Both fail identically on clean `main` (verified by stash A/B),
  so they are environment gaps, not regressions.
- `cargo clippy --workspace --all-targets` — zero clippy errors; the emitted
  warnings are pre-existing on `main`.
- `autospec validate` — stash A/B against clean `main`: the clean tree fails
  16 required catalog checks in this environment (same tooling gaps); this
  change adds **zero** new failures to that set (the one transient failure it
  introduced, `check_bats_negation_ratchet`, was remediated in-file by
  rewriting the two mid-body `!` negations to `if`-forms, and the checker and
  its 9-test self-suite are now green).
- `scripts/lint-implementation.sh --pre-commit --staged` — gate green
  (advisory `INFO:` findings only).
