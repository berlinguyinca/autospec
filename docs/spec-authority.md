# Spec authority currency (#3947)

The pipeline reads a spec as an instruction and never checked whether the
document that names itself authoritative is still the one in force. This
module makes spec-set currency an explicit, checkable claim.

## Currency markers

A document that claims authority (it carries an `## Authority` list, a
`## Supersedes` list, or a `## Superseded by` line) must also declare its
currency: a `## Version` line or a `## Supersedes` list. A document that
carries none of those sections makes no authority claim and is not bound by
the currency rules.

| Sections present | Verdict |
|---|---|
| `## Superseded by` | **Superseded** — dispatch refused |
| `## Version` or `## Supersedes`, no replacement named | **Current** |
| Authority claim, no version and no supersedes | **No currency marker** — reported, dispatch refused |

## Dispatch gate

`autospec dispatch freshness` runs the gate before the freshness verdict.
A staged document that is marked superseded, claims authority with no
currency marker, or participates in an authority conflict is refused with
exit 1 and a message naming the document and the reason:

```
SPEC-AUTHORITY issue 50 REFUSED: spec authority 50 is marked superseded (by inferweave-v2/README.md); dispatch against a superseded spec set is refused
```

The refusals are checked in severity order: superseded, then conflict, then
missing currency marker. With `--json` the refusal carries
`"authority_refused": true` and a `reason` field.

## Cross-program authority conflicts

When two documents claim authority over the same component, the conflict
surfaces as a reported refusal instead of a discovery made by accident while
tracing an unrelated dependency. Which program is current is a human
decision; the gate does not make it.

## Throughput by authority

`TaskRecord { issue, spec_authority, merged }` pairs each task with the spec
authority it derived from, and `throughput_by_authority` groups merged
throughput by that field, so a large volume number shows which program it
was actually pointed at instead of reading as direction.

## API

`autospec-core::spec::authority`:

- `parse_authority_doc(id, source) -> SpecAuthorityDoc`
- `SpecAuthorityDoc::claims_authority() -> bool`
- `currency_verdict(doc) -> CurrencyVerdict` (`Current` / `Superseded` / `NoCurrencyMarker`)
- `find_authority_conflicts(docs) -> Vec<AuthorityConflict>`
- `gate_dispatch(docs) -> DispatchGate` (`Admitted` / `Refused { refusal }`)
- `TaskRecord`, `throughput_by_authority(records) -> BTreeMap<String, u32>`
