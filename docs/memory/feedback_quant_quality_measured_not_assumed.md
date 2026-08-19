---
name: Quantisation quality — measure it, and separate infra failures from wrong answers
description: Q5_K_M and UD-Q8_K_XL scored 40/40 with zero disagreements; enabling reasoning and excluding 500s were both necessary to get a meaningful result
type: feedback
wing: synthesis
drawer_class: lesson
---
"Q8 beats Q5" drove the build choice in `llm/` for a long time without
ever being tested. `llm/linux-qwen38/scripts/compare-quants.py` tests it:
generated, exact-match-checkable items (arithmetic, instruction
following, code reasoning, in-context retrieval) asked of two live
endpoints, counting disagreements.

Local Q5_K_M (RTX 4090) vs UD-Q8_K_XL (96 GiB Blackwell), n=40,
temperature 0, reasoning on: **40/40 each, 0 disagreements.**

**Why:** three things had to be right before that number meant anything.

1. **Reasoning must be enabled.** With thinking off, *both* builds scored
   1/10 on code reasoning — and a category both fail 90% of has no power
   to discriminate. That was a defect in the instrument, not a finding.
   Qwen needs to reason.
2. **Infrastructure failures are not wrong answers.** Two earlier runs
   had the 8-bit build "losing" 8-11 items; every one was a `500` from a
   crashed server child, and on every item where it answered it agreed.
   Scoring those as quality would have produced the opposite conclusion.
3. **It has to replicate.** Three runs, different allocations and nodes.

**How to apply:** treat quantisation quality as measurable and measure it
before paying for capacity in its name. On this workload the 5-bit build
is not a compromise — spend a bigger card on **context and concurrency**,
which are large and measurable, not on quant quality, which here was not
measurable at all. State the resolution limit: at n=40 a difference under
~7 items is indistinguishable from noise, so this says "no visible
degradation", not "identical".

Related: [[reference_slurm_hpc_cluster]] for the crash that polluted the
first two runs.
