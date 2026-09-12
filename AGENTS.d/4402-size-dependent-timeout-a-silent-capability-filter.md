# A fixed timeout on a size-dependent operation is a silent capability filter, not a flaky check (issue #4402)

A worker read the served context from the model server with a single
`curl --max-time 10`, ten seconds after launching it. `/props` answers 503
until the weights are resident, and load time scales with model size. Against
the models that have ever registered on this fleet:

| model | weights | ever registered |
|---|---|---|
| qwen3.8-27b | 15.7 GB | yes |
| qwen3.8-27b-vision | 16.6 GB | yes |
| qwen3.8-flash-next | 107 GB | **never** |
| deepseek-v4-flash | 149 GB | **never** |
| glm-5.3-flash | 281 GB | **never** |

A clean threshold at roughly 30 GB. Every large model in the catalogue was
excluded from the fleet for as long as this code existed. Workers launched,
consumed GPUs for their full walltime, and were invisible.

The symptom presented as *model-specific flakiness*: "flash-next keeps having
problems", "compaction fails on the big models". Every investigation therefore
started inside the model — quantisation, KV layout, context sizing, the
runtime — because that is what the evidence appeared to point at. The actual
cause was one number in a registration path, and it discriminated by size
rather than by model. A bug that correlates with a property of the input looks
exactly like a property of the input.

- **A timeout on a size-dependent operation must be derived, not chosen.**
  Scale it from the input (bytes to load, rows to index) or from a bound that
  already governs the process. A constant is defensible only when the
  operation's duration genuinely does not depend on its input
  (`audit_timeout`). The tell is a clean step in the success pattern
  (`detect_size_filter`): every input below a boundary has succeeded, every
  input above it never has, and both sides are non-empty. The regression test
  reconstructs the incident (five models, 15.7/16.6 GB registered, 107/149/
  281 GB never) and asserts the boundary sits at roughly 30 GB.
- **A capability that is absent must be distinguishable from a capability that
  is failing.** Nothing reported "no worker for this model has ever
  registered". The fleet could not tell "we have no flash-next capacity" from
  "flash-next is unhealthy", and those need different responses
  (`classify_capacity`, `CapacityStatus::response`).
- **When a failure correlates with a property of the input, suspect the
  harness before the subject.** The first question for "model X keeps failing"
  is "what does the pipeline do differently for X?" — here, nothing, except
  take longer (`attribute_from_inputs`): when the failures separate cleanly on
  a measured input property, the discriminator is the harness, not the subject.

For specs and for planning across models: a spec covering a set of items with a
wide size range must state the range and require the implementation to be
tested at BOTH ends. A pipeline exercised only on the 15 GB model is not
evidence it works for the 281 GB one — and in this case the large end was
never exercised at all, in production, for months.

Checkable in `autospec_core::size_dependent_timeout` (`audit_timeout`,
`SizeSensitivity`, `TimeoutSource`, `TimeoutVerdict`, `SizedInput`,
`FilterThreshold`, `detect_size_filter`, `classify_capacity`,
`CapacityEvidence`, `CapacityStatus`, `attribute_failure`,
`attribute_from_inputs`, `Attribution`). Tests:
`crates/autospec-core/tests/size_dependent_timeout.rs`.
