# Rust supervised executor plan

## Goal

Replace the deferred `ExecutorRequest` placeholder with a claim-bound Rust
implementation child that emits exactly one typed terminal result and never
interprets shell commands.

## Scope

1. Add core request/result/replay types with strict identity validation.
2. Replace the CLI placeholder launch with direct argv, allowlisted environment,
   bounded output, timeout/process cleanup, and persisted invocation receipts.
3. Route only owner-verified terminal results into the existing claim protocol;
   require the #1697 pass receipt for success.
4. Add red/green tests for replay, timeout, command poisoning, and restart.

## Non-goals

The live QA/security producers and autonomous backlog deletion remain follow-up
issues (#2327 and #2076).

## Verification

Run the core executor tests, claim tiebreak tests, conductor command tests,
format, clippy, and the fast repository validator in a Rust 1.94 container.
