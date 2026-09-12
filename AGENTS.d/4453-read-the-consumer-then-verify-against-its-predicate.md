# A write to a control file is not a control action until verified against the reader's predicate (issue #4453)

To stop eight issues that were being re-dispatched forever (#4451), they were
appended to the dispatcher's hold list with the reason and a timestamp:

```
3550 repeatedly-dispatched-no-patch 2026-09-11T23:03:45-07:00
```

The dispatcher reads that file like this:

```sh
grep -qx "$n" "$L/autospec/queue-hold.txt" 2>/dev/null && continue
```

`-x` is a **whole-line** match. `3550` does not equal
`3550 repeatedly-dispatched-no-patch 2026-09-11T23:03:45-07:00`, so not one
of those holds would have taken effect. The file would have listed them, the
log would have said they were held, and the dispatcher would have kept
sending them to GPUs.

The report would have claimed a fixed leak that was still running — and the
next session would have had a written record saying the issues were held,
which is worse than no record at all. The write was only caught because the
consumer was read *before* reporting.

- **The rule: read the consumer, then verify against its predicate.**
  Before writing to a control file, read the code that consumes it, and
  verify the write against the consumer's *actual predicate* — not against
  the file's appearance. The file's appearance was exactly the trap here:
  the existing content was two bare numbers, the format was **visible in
  the file**, and a different one was appended anyway, because a richer
  line looked strictly more informative. Adding information to a control
  record is a natural instinct and it is exactly the mistake: **the
  reader's predicate, not the writer's intent, defines the format.**
- **A control file is an inter-process API call written in a file
  format.** Everything that makes an ordinary API call safe — a signature,
  a type, a compile error — is absent: an appended line is always
  syntactically valid; the only thing that can be wrong is the semantics,
  and nothing checks them. Operation success (the append cannot fail) is
  not the fact the operation was supposed to establish (the reader sees
  the record) — the same family as #4449 (silent false negatives) and
  #4434 (`GateVerdict`).
- **The one-command verification belongs in the procedure.** For this
  case it is the consumer's own command, run at the point of writing:

  ```sh
  grep -qx "$n" "$hold" && echo ok    # exactly what topup.sh will run
  ```

- **The stronger form: control files are not free-text append
  targets.** A typed writer that owns the format is what makes the
  writer's intent and the reader's predicate the same object. Where a
  file must stay plain text, the consumer's match is exercised by the
  writer as a **post-condition** — re-read the published file and run the
  consumer's predicate over it, so a write the reader cannot see fails at
  the point of writing. Annotation (reason, timestamp) moves to a
  sidecar keyed by issue: it must not have to corrupt the matched record
  to be recorded.

Checkable in `autospec_core::control_file` (`ControlRecord`,
`HoldRecord`, `ControlFile` — strict `parse`, `visible`,
`verify_visible`, atomic `write_verified` with the re-read
post-condition, `ControlFileError::InvisibleAfterWrite` /
`OffFormatLine`, `HoldSidecar`, `hold`, `release`).
Tests: `crates/autospec-core/tests/control_file.rs`, including the
regression that reconstructs the incident byte for byte (the exact
annotated line is invisible to the consumer's whole-line predicate, is
rejected by the typed parse at load time, and a `hold` against a
free-text hold list fails loudly without writing a byte) and the
post-condition check (`verify_visible` names every record the predicate
cannot see).
