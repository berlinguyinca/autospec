# Rust Executor Result Ingestion Design

## Goal

Add a Rust-only `autospec autonomous executor-result` protocol that accepts
strict executor evidence, records only claim-owner outcomes, and preserves the
current deferred foreground receipt until a real implementation executor exists.

## Context

`run-foreground` already owns the Rust queue, safety, claim, and conductor
state path. Its direct child currently receives only `--repo` and `--issue` and
returns a blocked deferred receipt. That compatibility protocol must remain
successful so the parent can persist a fail-closed paused state. The next
protocol must not treat a process exit as implementation success or allow a
foreign worker to overwrite a claim.

## Command contract

The existing compatibility invocation remains valid only with exactly the
required identity flags:

```text
autospec autonomous executor-result --repo OWNER/REPO --issue N
```

It prints the existing deferred JSON receipt and exits `0`. It does not write
claim state.

The explicit ingestion form requires every identity and outcome field:

```text
autospec autonomous executor-result \
  --repo OWNER/REPO --issue N --worker-id ID --branch NAME \
  --outcome succeeded|blocked|retryable [--pr N | --reason TEXT]
```

The parser accepts every flag at most once and rejects unknown flags, empty
strings, a non-positive issue or PR, and incomplete explicit input. `succeeded`
requires exactly `--pr`; it rejects `--reason`. `blocked` and `retryable`
require exactly `--reason`; they reject `--pr`. A partial mixture of the legacy
and explicit forms is malformed rather than silently defaulting to deferred.

Every result writes one compact JSON object to stdout. Exit codes are stable:

| Status | Exit | Meaning |
| --- | --- | --- |
| `accepted` | 0 | verified successful evidence was recorded, or the legacy deferred compatibility receipt was emitted |
| `retryable` | 10 | owner-verified retryable result was recorded and the claim remains held |
| `blocked` | 20 | owner-verified blocked result was recorded, or success evidence was unavailable and no claim changed |
| `malformed` | 2 | protocol validation failed before GitHub mutation |
| `ownership_lost` | 3 | worker, branch, or claim state no longer matches; no claim changed |

## Claim and evidence rules

`claim.rs` owns remote mutation. It exposes a narrow crate-visible
`record_executor_result` API that receives the repository, issue, worker,
branch, typed outcome, optional PR, and optional reason. The function must:

1. Read the selected run-state comment and require `state=claimed`, the same
   worker ID, the same branch, a fresh server lease timestamp, and no terminal
   merged marker before any mutation.
2. For `succeeded`, list open pull requests and accept the supplied PR only
   when it closes the target issue, contains exactly one Closeout report, and
   its `headRefName` equals the claimed branch. Evidence failure returns the
   typed blocked result before mutation.
3. Append a strict immutable executor-result receipt instead of patching the
   shared run-state lease. The receipt binds the exact owner, branch, outcome,
   PR, and generated receipt ID.
4. Re-read the receipt and the active run-state. Accept only if the exact
   receipt exists and ownership remains fresh and nonterminal; a takeover leaves
   inert evidence but returns `ownership_lost` without changing the lease.

No executor result releases a claim, merges a PR, or treats a bare successful
process exit as proof. A verified `succeeded` result is only the evidence that
allows the foreground state machine to move through its existing
`DispatchRecorded { Succeeded }` and `Reconciled` transition. A blocked result
remains claimed and the current foreground compatibility child keeps producing
the existing paused deferred receipt.

## Boundaries

- Use only the Rust executable, existing strict parsers, `gh` claim transport,
  and core conductor types. Add no dependency.
- Do not run `bash`, `sh`, `omx`, `autospec-run`, or any repository script.
- Do not edit or execute `scripts/autospec-autonomous-run-drain.sh`.
- The change does not launch an implementation agent or authorize PR merge.

## Test strategy

The CLI integration suite uses the existing fake `gh` fixture. It verifies
legacy deferred compatibility, strict rejection, each output/exit combination,
foreign worker and branch rejection without a state change, stale and terminal
claim rejection, branch-bound PR-closeout verification, takeover safety, and a
blocked owner result that remains
claimed. Core conductor tests retain the separate proof that success can leave
the selected dispatch only through `DispatchRecorded` and `Reconciled`.
