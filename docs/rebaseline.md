# Re-baseline in-flight work (issue #3708)

Rust core: `crates/autospec-core/src/rebaseline/`
(module `autospec_core::rebaseline`, split into `finalise`, `patch_meta`,
`verdict`).

Everything in the module is pure: no I/O, no git, no clock. The caller reads
the base and the trunk tip, and the module decides what the resulting drift
is worth.

## The incident

One issue was dispatched five times in about two hours and held **every time**.
Not one hold was for the agent's own quality — the last run produced
a correct implementation with the module properly declared. Every hold was
**base drift**: each field the patch was rejected for missing had been added
by a pull request that merged *while* the agent was running. Ten patches were
held in that session.

The measurements the issue records for the same window, and the exposure they
imply:

| Observation (issue #3708) | Value |
|---|---|
| Merges to `main` in the window | 7 in 2 h 50 m — one every ~24 min |
| Agent run lengths | 45, 7, 16, 13, 19 min — mean 20 min |
| Who did the merging | the supervisor, converting finished work |

A 20-minute mean run against a trunk that advances every 24 minutes is
**0.83 advances per run**, which is a **57% chance** (`1 - e^-0.83`) that at
least one merge lands before the patch is emitted. The longest run in that
window (45 min) sits at 1.88 advances and **85%**; the shortest (7 min) is
still 0.29 and **25%**. That is the issue's own reading — "the base moves
about as fast as an agent finishes, so a run of average length has roughly
even odds of being invalidated before its patch is read" — and every merge
causing it was produced by this pipeline itself. The loop is self-inflicted:
each merge invalidates every in-flight patch, so the cost of one merge is
multiplied by the number of patches in flight, and the merge rate is the
project's own success rate. Nothing in the pipeline makes the base move
slower. The only lever is work happening closer to the trunk.

The four rules below are the issue's fixes, minus prompt conversion
(newest-first, already shipped as #3698).

## The contract

### 1. An agent re-baselines before it finalises

[`pre_finalise_plan(drift)`](../crates/autospec-core/src/rebaseline/finalise.rs)
returns [`FinalisePlan::Emit`] only when the base is still the tip. Any other
drift returns the ordered plan

```text
fetch_trunk -> rebase onto <tip> -> rerun_gates
```

and the last step is the gates on purpose: a patch rebased onto a tree it was
never gated against carries a green receipt for a tree it will not land on.
[`receipt_currency(receipt_base, tip_sha)`] grades exactly that — `Valid` only
when the receipt's revision **is** the tip being judged against, `Void`
otherwise. This is the [`conversion_gate`] scope rule read from the other end:
a gate that never saw the current tree cannot have verified it, whatever its
exit code said.

### 2. A patch names the base it was verified against

[`PatchMeta`] is the sidecar the run emits with its patch:

```text
patch=out/issue-7/patch.diff base_sha=aaaa1111 tip_sha=bbbb2222 commits_behind=4 rebaselined=1 gates=check,test
```

With the base recorded, the converter knows how far behind the patch is
without applying it, and knows a clean apply onto a moved trunk is *staleness*
rather than conflict. Without it, each of those questions costs a GPU run to
re-ask. Rules the metadata enforces at write time, not at conversion time:

| Check | Result |
|---|---|
| `commits_behind > 0` and `rebaselined = 0` | `MetaViolation::EmittedStale { commits_behind }` |
| `rebaselined = 1` and `gates` empty | `MetaViolation::RebaselineWithoutGates` |
| `base_sha` absent from a parsed sidecar | hard parse error — never defaulted |
| ids and count disagree (`base == tip` with a non-zero count, or `base != tip` with zero) | `BaseDrift::measure` error |

A missing base is a **defect in the emit**, not a missing fact about the
patch: a pipeline that defaults it re-tests such a patch forever.

### 3. A `HELD` is a hypothesis, not a decision

[`hold_status(hold_tip, current_tip, commits_since, threshold)`] returns
[`HoldStatus::Hypothesis`] once the trunk has moved `threshold` commits
(default [`DEFAULT_HELD_HYPOTHESIS_COMMITS`] = 3) since the hold was recorded,
and [`HoldStatus::Decision`] while it has not. A hold recorded against a tree
that no longer exists is an experiment that is due — and re-running the test
costs the same run as re-citing the verdict while proving strictly more. A
hold against the current tip is a decision, and re-testing it buys the same
answer.

### 4. Drift is measured, not assumed

[`BaseDrift::measure(base_sha, tip_sha, commits_behind)`] cross-checks the
count against the two ids, because the two are independently observed and a
disagreement is a measurement bug, not a fact about the repository: a base
that *is* the tip is zero behind, a base that is *not* the tip is never zero
behind.

[`DriftExposure`] is the systemic version of the same measurement — run length
against mean merge interval — and it is the number to read before arguing
about any of the rules above:

```text
drift exposure 0.83 trunk advances per run, 57% of runs finalise onto a moved trunk (1200s run, 1440s merge interval): Invalidating
```

Classification is on the **probability** that at least one advance lands during
a run (`1 - e^-ratio`, merges at a constant mean rate), not on the mean ratio.
The mean understates the exposure badly in the region that matters: the trunk
in #3708 moved 0.83 times per run, which does not leave 17% of runs alone — it
leaves the runs with no merge in them, which is `e^-0.83` ≈ 43% of them.

| Class | Condition | Reading |
|---|---|---|
| `Sustainable` | p < 0.2 | drift is a rarity; a hold on drift is news |
| `Even` | 0.2 ≤ p < 0.5 | more than one run in five finalises onto a moved trunk; drift holds are background noise |
| `Invalidating` | p ≥ 0.5 | at least a coin flip; re-baselining is not an optimisation but the only way a patch lands |

The observed incident classifies as `Invalidating` **even though the mean is
below one advance per run** — which is precisely why reading the mean alone
called this configuration acceptable.

## What the module does not do

It does not run git, does not read the repository, does not decide whether to
re-dispatch a run. It turns two revision ids, a count, and (for exposure) two
durations into the verdict those numbers support, so that the shell and skill
layers — and the converter's hold/re-test decision — stop re-deriving it
inconsistently.

[`pre_finalise_plan(drift)`]: ../crates/autospec-core/src/rebaseline/finalise.rs
[`FinalisePlan::Emit`]: ../crates/autospec-core/src/rebaseline/finalise.rs
[`receipt_currency(receipt_base, tip_sha)`]: ../crates/autospec-core/src/rebaseline/finalise.rs
[`conversion_gate`]: conversion-gate.md
[`PatchMeta`]: ../crates/autospec-core/src/rebaseline/patch_meta.rs
[`hold_status(hold_tip, current_tip, commits_since, threshold)`]: ../crates/autospec-core/src/rebaseline/verdict.rs
[`HoldStatus::Hypothesis`]: ../crates/autospec-core/src/rebaseline/verdict.rs
[`HoldStatus::Decision`]: ../crates/autospec-core/src/rebaseline/verdict.rs
[`DEFAULT_HELD_HYPOTHESIS_COMMITS`]: ../crates/autospec-core/src/rebaseline/verdict.rs
[`BaseDrift::measure(base_sha, tip_sha, commits_behind)`]: ../crates/autospec-core/src/rebaseline/mod.rs
[`DriftExposure`]: ../crates/autospec-core/src/rebaseline/verdict.rs
