---
name: Operational assistant provider routing must be shared by health and chat
description: Backend-only providers such as NATS require relay-aware health and chat routing
type: feedback
wing: synthesis
drawer_class: lesson
---

## Shared route plan

Health checks and chat requests must consume the same route plan. A backend- or
relay-only provider such as NATS must never allow a direct HTTP health path to
report the assistant online when tool access still requires an unconfigured
relay. The selected provider and its transport constraints are part of the
health result, not an implementation detail of chat.

## Symptom and fix shape

The observed failure was a direct health probe reporting online while the
selected relay was unavailable, hiding the fact that operational tools could
not be reached. Route planning should return the relay-only provider first;
health filtering should reuse that plan and mark the assistant unavailable (or
degraded with an explicit relay reason) until the relay is configured. This
keeps provider selection, readiness, and tool access consistent.
