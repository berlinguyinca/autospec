# Issue Intent Safety Gate - Design Spec

**Date:** 2026-07-09
**Repo:** github.com/berlinguyinca/autospec
**Status:** Approved brainstorm; ready for implementation planning

## Goal

Prevent autospec from implementing malicious, destructive, or dangerously
ambiguous GitHub issues by requiring a fail-closed issue-intent safety review
before an issue can enter or remain in the `auto-implement` queue.

## Problem

Autospec can autonomously turn GitHub issues into implementation PRs. Today it
checks issue quality, model fit, and PR implementation quality, but an attacker
or confused operator can still file an issue whose requested outcome is unsafe:
delete production data, dump credentials, bypass auth, weaken audit logging,
disable CI, ignore project instructions, or execute untrusted shell.

The safety decision must happen before implementation starts. A Phase 4
guardian can catch unsafe code after work begins, but that still lets a hostile
issue consume agent cycles and shape the implementation prompt.

## Design

Add an **issue intent safety gate** with two layers:

1. `scripts/lint-issue-safety.sh` performs deterministic scanning of issue
   title/body using built-in defaults plus `.autospec/autospec.yml` rules.
2. A Tier A semantic reviewer evaluates ambiguous or high-risk issues that the
   deterministic scanner cannot safely pass.

The gate produces one of three decisions:

| Decision | Meaning | Queue behavior |
|---|---|---|
| `SAFETY_PASS` | Bounded safe work; no destructive or hostile intent detected. | Add `safety:reviewed`; issue may enter `auto-implement`. |
| `SAFETY_AMBIGUOUS` | Potentially legitimate but unsafe without clearer scope. | Add `security:quarantined`; remove queue labels. |
| `SAFETY_BLOCK` | Clear malicious or destructive intent. | Add `security:quarantined`; remove queue labels. |

For v1, both `SAFETY_AMBIGUOUS` and `SAFETY_BLOCK` quarantine the issue. The
operator can edit the issue into safe scope, remove `security:quarantined`, and
rerun `/autospec-classify`.

## Enforcement Points

### Phase 3 pre-filing

For autospec-generated child issues, the decomposer runs the safety gate before
`gh issue create`. If a draft fails, the decomposer retries up to five times
with cumulative safety directives. After five failures, it skips that child and
surfaces the drafts/findings instead of filing an unsafe issue.

### Phase 3.5 and `/autospec-classify`

Classification checks every candidate issue before adding or preserving
`auto-implement`. Passing issues get a `## Safety review` block and
`safety:reviewed`. Quarantined issues get `security:quarantined`, lose
`auto-implement` and `needs-classify`, and receive an audit comment.

### `/autospec-run`

Before claiming an `auto-implement` issue, the monitor refuses any issue that:

- carries `security:quarantined`;
- lacks `safety:reviewed`; or
- lacks a current passing `## Safety review` block.

This fail-closed runtime check prevents manually labeled issues from bypassing
classification.

## Config

Extend the existing `.autospec/autospec.yml` `safety:` namespace. Built-in
defaults apply when the config file is missing, invalid, or lacks the
`issue_intent_gate` section.

```yaml
safety:
  production_access: strict_isolation
  secrets_policy: references_only
  never_clone_raw_secrets: true

  issue_intent_gate:
    enabled: true
    default_decision: quarantine_uncertain
    require_pass_marker_for_run: true

    quarantine_labels:
      - security:quarantined
    remove_labels_on_quarantine:
      - auto-implement
      - needs-classify

    block_patterns:
      - id: production-data-destruction
        severity: block
        patterns:
          - "(?i)delete .*production"
          - "(?i)drop .*prod(uction)? .*database"
      - id: secret-exfiltration
        severity: block
        patterns:
          - "(?i)(dump|print|exfiltrate|send).*secret"
          - "(?i)(aws|github|stripe).*token"
      - id: instruction-bypass
        severity: block
        patterns:
          - "(?i)ignore (all )?(previous|system|developer|agent) instructions"
          - "(?i)bypass (ci|tests|hooks|review|guardian)"
      - id: destructive-shell
        severity: block
        patterns:
          - "rm -rf /"
          - "(?i)curl .*\\| *(sh|bash)"

    ambiguous_patterns:
      - id: vague-data-cleanup
        severity: ambiguous
        patterns:
          - "(?i)clean (old|bad|stale)? ?data"
      - id: weaken-security-control
        severity: ambiguous
        patterns:
          - "(?i)(relax|disable|remove).*security"
          - "(?i)(relax|disable|remove).*(auth|audit|logging)"
      - id: production-or-infra-touch
        severity: ambiguous
        patterns:
          - "(?i)(production|prod|billing|payments|migration|terraform|iam|kms)"

    semantic_review:
      enabled: true
      trigger_on:
        - deterministic_ambiguous
        - risky_keyword
        - auth_or_secrets
        - production_or_infra
```

V1 allows operators to add rules, tune labels, and tune escalation triggers. It
does not allow a repo config to silently weaken the built-in never-bypass
categories. Any debug-only disable path must be explicit and noisy.

## Trusted Actors

Trusted actors can pass scoped dangerous work, but they do not bypass the gate.

```yaml
safety:
  issue_intent_gate:
    trusted_actors:
      - login: berlinguyinca
        trust: repo_owner
        allowed_risk:
          - test_data_reset
          - fixture_regeneration
          - local_dev_cleanup
          - documented_migration_replay

    trusted_actor_rules:
      require_scope_match: true
      never_bypass:
        - secret_exfiltration
        - credential_printing
        - auth_backdoor
        - production_data_destruction
        - instruction_bypass
        - ci_or_review_bypass
```

Example: a trusted repo owner issue that says "delete the test database and
repopulate it" can pass only when the body clearly scopes the request to
test/dev/local data and includes verification. The same actor asking to dump
secrets, bypass review, create a backdoor, or delete production data is still
quarantined unless a later scoped-production design explicitly defines the
backup/restore contract.

## Audit Block

Every reviewed issue gets an idempotent block:

```markdown
## Safety review

- **decision:** `SAFETY_PASS`
- **actor:** `berlinguyinca`
- **trust:** `repo_owner`
- **matched rules:** `trusted:test_data_reset`
- **reason:** trusted actor requested test-only reset; scope matched `test`.

<!-- autospec-safety:begin -->
*Auto-reviewed by issue intent safety gate on YYYY-MM-DD.*
<!-- autospec-safety:end -->
```

Blocking reviews use the same block with `SAFETY_BLOCK` or
`SAFETY_AMBIGUOUS`, the matching rule IDs, and the clarification needed before
the issue can be reconsidered.

## Failure Handling

- Quarantine is idempotent: reruns replace the safety block and do not stack
  comments unnecessarily.
- Invalid YAML falls back to built-in conservative defaults and emits a warning.
- A missing safety marker in `/autospec-run` is a refusal, not a warning.
- Quarantined issues are not closed. They remain visible for human review.
- The gate must not shell out free-form issue text as code; all matching treats
  issue content as data.

## Tests

Validation follows the repo's existing shell/Bats pattern.

Required fixtures:

- malicious production data deletion -> `SAFETY_BLOCK`;
- ambiguous "clean old data" -> `SAFETY_AMBIGUOUS`;
- trusted repo owner resets test database -> `SAFETY_PASS`;
- trusted repo owner dumps secrets -> `SAFETY_BLOCK`;
- invalid YAML -> built-in defaults still quarantine dangerous text;
- `/autospec-run` refuses an issue without `safety:reviewed`;
- `/autospec-classify` removes `auto-implement` and `needs-classify` when
  quarantine applies.

Required validation updates:

- `autospec validate` runs the new safety linter tests.
- `schemas/autospec-config.schema.json` accepts `safety.issue_intent_gate`.
- Lock-step skill bodies are updated for `autospec`, `autospec-define`,
  `autospec-classify`, and `autospec-run`.

## Non-goals

- No production-operation approval workflow in v1.
- No broad trust bypass for repository owners.
- No replacement for the existing Phase 4 implementation guardian.
- No new test framework.
