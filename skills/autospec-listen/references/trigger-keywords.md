# Trigger keywords

Canonical list of trigger phrases the `autospec-listen` skill watches for in
chat. These are the verbatim phrases from spec §4.4. Matching rules:

- Case-insensitive.
- Word-boundary anchored (`\b<phrase>\b`).
- Multi-word phrases match contiguously (whitespace tolerant).
- Bare nouns are NOT triggers — only the phrases below classify.

## Issue triggers

- `file an issue`
- `file this as an issue`
- `new issue`
- `open an issue`
- `create a ticket`
- `make an issue`

## Spec triggers

- `write a spec`
- `design spec`
- `new spec`
- `start a spec`
- `write a design spec`

## Notes

Bare nouns are NOT triggers. The following do NOT classify:

- `issue` (alone)
- `spec` (alone)
- `ticket` (alone)

Phrases like "the issue here is...", "the spec says...", or "this ticket
needs review" must classify as `none`. Only the imperative-verb phrases above
fire the listener.

Matching is case-insensitive and word-boundary anchored — embedded triggers
inside larger sentences DO classify (e.g. "could you file an issue for
that?" classifies as `issue`).
