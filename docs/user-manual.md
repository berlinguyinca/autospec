# Autospec user manual

A narrative walkthrough of the autospec skills. Use this manual
as the "what does this do, when do I run it, what do I expect to see"
companion to the per-skill `README.md` and `SKILL.md` files.

The skills are presented in the order an end-to-end ship goes through
them: umbrella → existing-spec split → planning → classification →
implementation → passive listener.

## autospec

### What it does

`/autospec` is the umbrella end-to-end skill: a single feature request
goes in and a series of merged PRs (closing the auto-generated issues)
comes out. Internally it runs Phase 0 (bootstrap repo if missing),
Phase 1 (investigate), Phase 2 (brainstorm + design), Phase 3
(decompose into linked GitHub issues), Phase 3.5 (review-and-label
with `ctx:*`/`reasoning:*`), the Phase 3 pre-impl gate (default
`run`), then Phase 4–6 (background autonomous monitor + status
updates + final report).

### When to use it

Reach for `/autospec` when you have a clearly-bounded feature you want
shipped end-to-end with admin-merge authority, and you intend to be
notified when the implementation half completes — not when you want
to author a spec by hand.

### Example output

```
/autospec "add OIDC support behind a feature flag"
Phase 0: bootstrap — repo present, skipping
Phase 1: investigate — staged 6 files, 2 spec sections
Phase 2: brainstorm — 4 design questions answered
Phase 3: decompose — 9 child issues filed (#102…#110)
Phase 3.5: classify — labeled with ctx:* and reasoning:*
Spec written, 9 issues filed. Start /autospec-run now, defer to your
external daemon, or keep refining? [run / defer / refine] (default: run)
> run
Phase 4: monitor — processing #102…
```

## autospec-define

### What it does

`/autospec-define` is the planning half of the umbrella: it runs
Phases 0–3.5 only and stops at the Phase 3 pre-impl gate (default
`defer`). It produces a populated `auto-implement` queue but does NOT
launch implementation.

### When to use it

Use `/autospec-define` when you want a human to review the spec and
the issue decomposition before the implementer touches code. Pair it
with `/autospec-run` when the review passes.

### Example output

```
/autospec-define "decouple billing from auth service"
Phase 0–3.5 complete. 7 issues filed (#118…#124).
Spec written, 7 issues filed. Start /autospec-run now, defer to your
external daemon, or keep refining? [run / defer / refine] (default: defer)
> defer
Issues are ready. Your external monitor will pick them up. Exiting.
```

## autospec-split

### What it does

`/autospec-split` is the existing-spec shortcut: it selects a tracked
`docs/specs/*.md` file, skips Phases 1–2, decomposes that spec into an
EPIC plus `auto-implement` child issues, runs Phase 3.5 review-and-label,
then stops with the `/autospec-run` handoff.

### When to use it

Use `/autospec-split` when the design spec already exists on `origin/main`
and you only need to materialize it into GitHub issues. Use
`/autospec-define` instead when the spec still needs to be written or
landed.

### Example output

```
/autospec-split split latest spec
selected docs/specs/2026-05-01-example-design.md
Phase 3: decompose — 6 child issues filed (#130…#135)
Phase 3.5: classify — labeled with ctx:* and reasoning:*
Phase 3 complete. Run /autospec-run --profile <name> to begin implementation.
```

## autospec-run

### What it does

`/autospec-run` is the implementation half: it consumes any open
`auto-implement` issues whose `Depends-on` graph is satisfied,
processes them one at a time through a TDD inner loop, opens a PR per
issue, self-reviews, and admin-merges on `LGTM`.

### When to use it

Use `/autospec-run` to start implementation against an already-filed
queue (filed by `/autospec-define`, by hand, or by a previous monitor
run that was `defer`-red). The optional `--profile <name>` flag
filters issues to those whose `ctx:*`/`reasoning:*` labels are
allowed by the named profile in `~/.autospec/model-profiles.yml`.

### Example output

```
/autospec-run --profile claude-sonnet-cloud
auto-implement queue: 7 open, 7 in-profile
processing #118 — feat(billing): extract billing client
PR #119 opened, self-review LGTM, admin-merged
processing #120 — feat(auth): drop billing dependency
...
```

## autospec-classify

### What it does

`/autospec-classify` retro-applies the Phase 3.5 model-fit rubric to
already-existing issues. It walks both `auto-implement` AND
`needs-classify` issues, picks the smallest matching `ctx:*` and
`reasoning:*` labels, inserts a `## Model fit` block in the body, and
(for `needs-classify` issues) transitions the label to
`auto-implement`. Optional `--apply-boards` routes each issue onto a
Projects board per `~/.autospec/project-map.yml`.

### When to use it

Use `/autospec-classify` to (a) backfill `ctx:*`/`reasoning:*` labels
on issues that pre-date Phase 3.5, (b) sweep the `needs-classify`
backlog filed by `/autospec-listen` so those issues become eligible
for `/autospec-run`.

### Example output

```
/autospec-classify
queue: 4 auto-implement, 6 needs-classify
classified #62 -> ctx:64k reasoning:medium (transitioned needs-classify -> auto-implement)
classified #63 -> ctx:32k reasoning:shallow (transitioned)
...
Phase 3.5 summary: classified 10, ctx:32k=4 ctx:64k=6, reasoning:shallow=3 medium=7
```

## autospec-listen

### What it does

`/autospec-listen` is a passive conversation listener. It watches an
agent session for canonical trigger phrases ("file an issue",
"new issue", "open an issue", "create a ticket", "write a spec",
"design spec", "new spec", "start a spec"). On a match it either
drafts a GitHub issue body for confirmation (issue trigger) or hands
off to `/autospec-define` (spec trigger). Bare nouns ("issue",
"spec", "ticket") are NOT triggers.

### When to use it

Use `/autospec-listen` to capture in-the-moment intent without making
the user remember a slash command. Issues filed by the listener
carry `needs-classify` and become eligible for `/autospec-run` after
`/autospec-classify` sweeps them.

### Example output

```
user: That regression in PrometheusRegistry.collect is bad — file an issue.
listener: Detected issue trigger. Draft body:
  Goal: Fix PrometheusRegistry.collect latency under high cardinality.
  Context: 800 routes × 6 methods × 12 buckets = ~57k samples per scrape...
  Suggested AC: [ ] /metrics responds in <1s p99 with 800 routes.
  File this issue? [yes / no]
> yes
Filed #128 with label needs-classify.
```
