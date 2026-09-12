# A zero from a search over a source the tool does not index is not evidence of absence (issue #4344)

Determining which of two architecture programs owned a component, the
investigator asked GitHub code search what the candidate repository
contained:

```text
'v1/models'        -> 0 hits
'chat/completions' -> 0 hits
'axum'             -> 0 hits
'TcpListener'      -> 0 hits
```

Four zeros, consistent with each other and with the hypothesis under test —
that the repository had no HTTP server and therefore did not own the
gateway. The conclusion was about to be that a second gateway should be
built in the *other* repository, beside one that already existed: precisely
the duplication the investigation existed to prevent, justified by evidence
that looked clean and was measuring nothing.

Reading the directory listing directly, the repository contains
`openai_api.rs`, `openai_runtime.rs`, `proxy.rs`, `relay.rs`, `transport.rs`,
`lease_router.rs`, `routing.rs`, `scheduler.rs`, `daemon.rs`,
`admin_http.rs`, `connectivity.rs`, `receipt_store.rs`, `dht.rs`, `gossip.rs`,
`discovery.rs`, plus a deployable `[[bin]]`. **GitHub does not index private
repositories for code search. It returns zero rather than an error**, so an
unavailable index and an empty repository produce the same answer.

A failed command announces itself. A search that returns no results looks
like a successful measurement of an empty set, and the shape of the output
is identical either way. It is the same defect as an empty CI log while the
API explains it is refusing to print escape sequences, and the same as an
empty `cron-*.log` because the writer logs elsewhere. A confident negative
from a tool whose coverage you have not verified is the most expensive kind
of wrong answer, because it terminates the investigation.

- **Before trusting a negative, confirm the tool can produce a positive
  over that source.** Search for something certain to be present. If that
  also returns zero, the index is the finding — the investigation's result
  is "the tool cannot see this source", not "the source is empty".
- **Prefer enumeration over search when the question is "does X exist
  here".** A directory listing cannot be partially indexed: it is present,
  or it errors.
- **Mutually consistent zeros from one tool are one observation, not
  four.** They share a failure mode, so agreement between them carries no
  independent weight; the four zeros that looked most convincing each
  independently confirmed the others and shared a single cause.
- **Record the coverage limits of investigative tools where they will be
  read.** "GitHub code search does not index private repositories and
  returns zero, not an error" is a fact worth stating once rather than
  rediscovering.

Checkable in `autospec_core::negative_evidence` (`classify`,
`GroupVerdict`, `NegativeReport::weight` — the number of independent
`(tool, source)` groups, never the number of queries, `Control`,
`CoverageLimit`). Tests: `crates/autospec-core/tests/negative_evidence.rs`,
including the incident end-to-end: four consistent zeros over a source the
tool does not index are one untrusted observation; the control that was
never run makes the index the finding; the directory listing that was never
read makes the negative hold.
