## A test's verdict must not depend on which targets ran beside it

`executor_supervision_descendant_capture_reserves_descriptor_headroom`
asserted on a fresh `free_descriptor_slots()` re-read after the fail-closed
capture. The budget check fires at 32 free slots, so any descriptor a sibling
test in the same binary opened between the check and the re-read showed up as
31 — and the test's verdict depended on which targets ran beside it: green in
a 225-target workspace run, red when the target ran alone, same commit
(#4557; observed on CI as "saw 31").

The assertion now reads the number the budget check itself saw — embedded in
the error and read atomically at the moment of the check — and pins the
fail-closed property from the check's own value. The headroom the reserve
exists for is proven the way it is real: after the fail-closed stop the
process recaptures the whole tree unconstrained (the EMFILE-proof, which the
old absolute re-read was trying, and failing, to show). The ceiling is now
derived from `DESCENDANT_DESCRIPTOR_RESERVE` instead of the magic 36.
