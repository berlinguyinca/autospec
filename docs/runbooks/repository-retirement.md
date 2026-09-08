# Runbook — Repository retirement & reimplementation

Governing rule: **working software is a specification with no readers and an
expiry date.** Archiving is not free; it is a decision to delete everything the
repository knew that was not code. Code survives archival on its own. The
program — phase order, topology, deferred gaps, rationale — does not.

Three consequences, enforced as checklist steps below:

1. **Extract before archiving.** Program state, topology, deferred decisions
   and rationale are relocated to a named home *first*; the archive happens
   *after*.
2. **Specify from a running original.** When a second implementation is
   planned, the first implementation's design decisions are written down
   **while the original still runs**, with the running system cited as the
   evidence. Afterwards it is archaeology.
3. **Rationale belongs where the decision is used.** A comment in the file
   that implements a rule reaches only its readers; a cross-cutting invariant
   lives in [`../invariants.md`](../invariants.md) and names the components it
   binds.

## Checklist — retiring a repository

Complete every step and commit the results to the named home **before**
running `gh repo archive` (or the equivalent). The archive action itself
stays behind the destructive-action confirmation gate
(`scripts/autospec-autonomy-gate.sh`), but the gate does NOT verify these
steps — this checklist does.

- [ ] **Name the home.** The retirement PR/issue names the destination:
      `docs/handoffs/YYYY-MM-DD-<repo-slug>-retirement.md` in the successor
      repository (or the successor's `docs/memory/`), or a file in this
      repository if no successor exists.
- [ ] **Program state relocated.** Intended sequence / phase order,
      current position within it, and any in-flight state, written from the
      roadmap or tracker (not reconstructed from memory).
- [ ] **Topology relocated.** Deployment machine assignments, network shape,
      and environment layout — as a table or diagram, with each component
      named.
- [ ] **Deferred decisions relocated.** Known gaps and deliberately deferred
      work, each with the reason it was deferred (open issues linked where
      they still exist).
- [ ] **Rationale relocated.** Non-obvious design decisions with their why.
      For every decision, either a link to its spec/handoff or a paragraph
      of rationale copied in.
- [ ] **Cross-cutting invariants migrated.** Any invariant the archived
      component binds (see [`../invariants.md`](../invariants.md)) is copied
      into the successor's invariants document, with the successor component
      added to the components-bound list.
- [ ] **Reimplementation brief written** (only if a second implementation is
      planned) — see below.
- [ ] **Evidence check.** A reader who has never seen the archived repository
      can answer: what was the phase order, where did each piece run, what is
      known-but-deferred, and why were the non-obvious decisions made? If any
      answer requires opening the archived repo, the extraction is incomplete.

Only after all boxes are ticked: archive the repository.

## Brief — reimplementing a running component

When component B is being planned as a reimplementation of running component
A, component A's design decisions are documented **while A still runs**:

- [ ] Each non-obvious decision in A is written down with its rationale,
      in the reimplementation spec or the invariants document.
- [ ] Each entry cites the running system as evidence: the component, the
      revision/commit it was observed at, and the command or probe that
      demonstrates the behavior (e.g. the request/response that proves an
      auth gate, the metric that proves a caching choice).
- [ ] Deliberate non-decisions are recorded too — things A *chose not* to do
      (e.g. a gate deliberately left unmerged, a field deliberately not
      tracked) — with the reason.
- [ ] The brief names the file/section where a future implementer of B
      should find each decision.

Entries written after A stops running are marked `archaeology:` and treated
as lower-confidence by the implementer of B.

## Invariants — home outside the implementing file

Cross-cutting invariants live in [`../invariants.md`](../invariants.md), not
only in comments of the file that implements them. Every entry must:

- state the invariant in one or two sentences,
- **name the components it binds** (the file/component is the *implementing*
  location; the components-bound list is the *reach*),
- link to where it is implemented and where it was decided.

A new component that is bound by an existing invariant must be added to the
invariant's components-bound list in the same PR that introduces the
component.
