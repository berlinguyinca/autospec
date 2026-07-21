---
name: Generic failure diagnostics flow
description: Diagnose broad failure questions from current scoped evidence before asking the user to choose a tool
type: feedback
wing: synthesis
drawer_class: lesson
---

## Start from the current scope

When a user asks broadly why resources failed, automatically run read-only failure
diagnostics against the current resource and scope. Do not ask the user to choose a
diagnostic tool, and do not guess from a terse state string.

## Separate state from reason

Treat entity state as lifecycle position, not root cause. Retrieve the failure reason,
exit code, and timestamp independently. A state such as `FAILED` says what happened to
the entity; fields such as `OOM`, `exit status 137`, or a recorded exception explain why.

## Group before attributing

Group failures by parent unit, workflow, reason, and time window before assuming each
record represents an independent data problem. Correlated children can be symptoms of
one parent failure or shared infrastructure event.

## Prefer governed evidence

Use the governed database or supported log-tail interface before reaching into raw
object storage. Keep the query read-only and bounded to the current scope.

For logs, find high-signal anchors such as `Traceback`, `Caused by`, `panic`, `OOM`, and
`exit status`, then return a bounded excerpt around the best anchor. A log head commonly
contains startup noise and omits the failure itself.

## Acceptance shape

A broad “why did these fail?” request triggers scoped, read-only diagnostics that report
state separately from reason, summarize correlated failure groups, and cite bounded
anchor-based log evidence. Ask for clarification only when the current resource or scope
cannot be determined safely.
