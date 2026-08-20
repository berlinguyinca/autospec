# Design

## Source of truth

- Status: Active
- Last refreshed: 2026-08-20
- Primary product surfaces: operational overview, model/service catalog, rendered service guide, server fleet, usage, health
- Evidence reviewed: `web/index.html`, `scripts/gateway.py`, `scripts/collect-stats.py`, `scripts/publicview.py`, `scripts/keystore.py`, and the live `/api/stats` and `/api/servers` payloads

## Brand

- Personality: restrained, technical, candid, and calm under load
- Trust signals: measured values, explicit time windows, sample counts, and unknown states that remain unknown
- Avoid: decorative charts, fabricated zeroes, ambiguous “online” badges, and claims that capability means residency

## Product goals

- Goals: answer what is loaded, which quantized artifact each deployment serves, how to select a model and deployment/GPU route, where work is running, which authenticated principals and credentials have requests active, how busy each server and GPU is, and what throughput has been observed
- Non-goals: historical time-series analytics, alerting, GPU-process attribution, or observing clients that bypass this gateway
- Success signals: the landing view answers the five operational questions without opening another tab, while the existing detail pages remain authoritative drill-downs

## Personas and jobs

- Primary personas: node operator, lab member choosing a model, and client owner diagnosing slow work
- User jobs: compare quantization/context/capability tradeoffs, select the balanced route or a specific deployment, confirm availability, distinguish queueing from slow inference, see loaded versus merely available models, and identify current gateway activity
- Key contexts of use: desktop operations, narrow laptop screens, and an unauthenticated public status check

## Information architecture

- Primary navigation: Operations, Models, Servers, Guide, Connect, Chat, Keys, Usage, Health
- Core routes/screens: `/#overview` remains the stable landing route and is labeled Operations
- Content hierarchy: fleet state, active work, loaded models, server load, GPUs, then local queue detail

## Design principles

- Truth before density: unsupported or stale telemetry renders as unknown, never inferred
- Live and historical signals must look different and carry their time window
- Public views expose aggregate load; members see only their own active identity details, and administrators may see every active identity
- Detail tabs remain available; the landing view summarizes rather than duplicates their controls
- Tradeoffs: the overview joins independently refreshed APIs, so it shows each source's freshness instead of pretending to be an atomic snapshot
- Model ids choose weights and serving profiles; server routes choose deployments. A GPU is independently selectable only when it is exposed as its own runtime/server id.

## Visual language

- Color: reuse the existing neutral palette and semantic accent, success, and warning colors
- Typography: reuse system UI and monospace stacks
- Spacing/layout rhythm: 12 px grid gaps, compact metric cards, and scrollable tables on narrow screens
- Shape/radius/elevation: reuse existing cards, badges, bars, and border treatments
- Motion: only existing bar-width transitions; no animated charts
- Imagery/iconography: none; operational text and measured values carry the interface

## Components

- Existing components to reuse: `.grid`, `.card`, `.badge`, `.bar`, `.scroll`, server badges, GPU cards, and problem rows
- New/changed components: operations KPI strip, loaded-model rows, service-catalog table, routing-flow diagram, deployment chooser, server-load table, and active-request/principal panel
- Variants and states: live, measured baseline, unknown, stale, empty, public aggregate, and authenticated detail
- Token/component ownership: `web/index.html`; no new design-system layer or dependency

## Accessibility

- Target standard: WCAG 2.1 AA within the existing page architecture
- Keyboard/focus behavior: preserve hash-routed tab keyboard navigation and semantic tables
- Contrast/readability: reuse tested semantic colors and include text labels for every state
- Screen-reader semantics: operations updates use bounded `aria-live` regions; tables retain headings
- Reduced motion and sensory considerations: no essential information depends on motion or color

## Responsive behavior

- Supported breakpoints/devices: existing desktop layout and the current 880 px single-column breakpoint
- Layout adaptations: metric grids collapse naturally; wide model/server/client tables scroll inside cards
- Touch/hover differences: no hover-only information or precision controls

## Interaction states

- Loading: em dashes and concise “waiting for fleet telemetry” copy
- Empty: distinguish idle, no recorded work, and unsupported telemetry
- Error: keep the last safe snapshot and show source-specific unavailability without blocking other panels
- Success: loaded and online badges use text plus color
- Disabled: existing gated tabs remain visibly unavailable until sign-in
- Offline/slow network: each request body is drained, sources fail independently, and the page remains navigable

## Content voice

- Tone: plain, specific, and operational
- Terminology: loaded, available, quantization variant, serving profile, deployment, active, queued, live decode, measured prefill/decode, and unknown
- Microcopy rules: always name the observation window and never use “currently” for a historical aggregate

## Implementation constraints

- Framework/styling system: framework-free HTML, CSS, and JavaScript plus Python standard-library services
- Design-token constraints: extend existing CSS variables and components only
- Performance constraints: no new request-path database scans; activity tracking is bounded without undercounting; probes stay on background timers; browser polls never overlap
- Compatibility constraints: generic OpenAI-compatible servers may not report residency or quantization, so both remain nullable and provenance-bearing
- Test/screenshot expectations: test API projection and concurrency behavior first, run both node suites together, run structural validation, and visually inspect desktop and narrow layouts

## Open questions

- [ ] A future time-series store may add p50/p95 latency and historical GPU charts; it is outside this change because the current event schema cannot support them honestly.
- [ ] Direct callers that bypass this gateway remain invisible; the UI must keep that boundary in its active-request copy.
