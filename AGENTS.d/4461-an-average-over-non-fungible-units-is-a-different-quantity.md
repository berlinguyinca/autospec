# An average over non-substitutable units is a different quantity, not an approximation (issue #4461)

The fleet autoscaler decided scale-up from one number —
`31% busy (27/87 over 3 samples), target 75%, running=5 want=5` — and said
no for four consecutive runs. Throughout that window `deepseek-v4-flash`
sat at **1/1 slots with a deferred queue of 2**: fully saturated, turning
requests away. Its contribution to the fleet average is one slot in 87;
its saturation moves the fleet number by about one point. Lowering the
75% threshold does not fix it: a 1-slot model at 100% utilisation can
raise a fleet average of 87 slots by at most 1.1 percentage points, so no
threshold that is meaningful for the fleet can ever be crossed by that
model's saturation alone. And the masking is *worst where it matters
most*: slot counts come from a planner that gives large models fewer
slots, so the biggest, scarcest, most expensive-to-provision models are
exactly the ones whose saturation is most thoroughly averaged away. The
signal degrades in proportion to how much you need it.

The units are not interchangeable. A request for `deepseek-v4-flash`
cannot be served by a `qwen3.8-27b` slot, so "27/87 busy" is not a
statement about capacity available to any actual request. The fleet can be
simultaneously over-provisioned in aggregate and under-provisioned for
every model that has demand — which is the state it was in.

- **A capacity or health decision must be made at the granularity at which
  the resource is actually allocatable.** For the scaler that is
  per-model: `requests_deferred` per model, sustained across samples, is
  the trigger, and the fleet-wide occupancy is reported alongside for
  context rather than used as the decision. The decision line names the
  model and its deferred count.
- **Wherever a threshold is applied to an average, the code records what
  population the average covers and asserts that the members are
  substitutable.** Where they are not, the check belongs on the
  individual — the aggregate is then a different quantity that happens to
  share its name, not a coarse version of the truth.
- **The error grows as the scarce unit gets scarcer.** The width of the
  band a global threshold could use to detect one unit's saturation is
  that unit's share of the fleet (`max_average_shift`): 1.1 points for
  1 of 87, 11 for 1 of 10. An average over non-fungible units is
  therefore not "approximately right"; it is a different quantity, and
  its distance from the decision-relevant truth is largest exactly where
  the decision matters most.
- **Not satisfiable by tuning.** The fix is not a threshold value. No
  global threshold, at any value, can reproduce the per-unit decision
  over non-substitutable units — the aggregate check is refused
  structurally, not merely below the bar.

## The enumeration (AC1): decisions in this workspace driven by
"percent busy" style aggregates, and what their units are

| Decision | Where | Units of the average | Verdict |
|---|---|---|---|
| Fleet agent capacity (`resolve_capacity`, `target_initial_width`) | `autospec_core::planning::capacity` | agents — any agent can decompose any issue | fungible; aggregate valid |
| Fleet reconcile status (`want=N up=N pending=N`, suspension at a per-claim start-failure rate) | `autospec_core::fleet_dispatch` | worker slots — any worker serves any claim; the decision is per-claim, never on a fleet average | fungible; decision already per-unit |
| KV over-commitment (`KvConfig::classify`, `fundable_slots`) | `autospec_core::kv_overcommit` | slots of one worker sharing one pool — fungible within the worker; the decision is per-worker, and `fleet_audit` is a separate report, not a threshold on a fleet average | fungible; decision already per-unit |
| Systemic failure signature (`share_percent` + `--threshold-percent`) | `autospec_core::failure_signatures`, `autospec doctor failures` | runs of the *same* failure signature — identical units | fungible; aggregate valid |
| GPU-hour cost share (`--threshold-percent`) | `autospec-cli` `cost` | GPU-hours — an identical fungible resource, flagged per status | fungible; aggregate valid |
| Baseline load attribution (`busy_slots`/`total_slots`) | `autospec_core::workload_validation` | slots — but the code refuses a parameter claim argued from a number taken under load and names the busy-slot population | no aggregate-threshold decision; already per-unit-aware |
| Offer scoring (`(1.0 − utilization) × 0.75` term) | `autospec_core::aar::inferweave` | per-offer ranking score, not a threshold on a population average | per-unit; no aggregate decision |
| Pipeline constraint (`identify_constraint`) | `autospec_core::execution::stage_throughput` | per-stage throughput comparison (lowest rate wins, not highest utilisation) | per-unit; no aggregate decision |
| Fleet autoscaler (`31% busy (27/87 over 3 samples), target 75%`) | the autospec-node repo (fleet autoscaler) | models that cannot serve each other's requests — **not** substitutable | moved to per-model sustained `requests_deferred` by this issue; the checkable form of the rule lives in this crate |

Checkable in `autospec_core::aggregate_granularity`
(`UnitObservation`, `FleetSample`, `Fungibility`, `aggregate_threshold`,
`max_average_shift`, `decide_scale_up`, `ScaleUpDecision`).
Tests: `crates/autospec-core/tests/aggregate_granularity.rs`, including
the regression that reconstructs the incident's shape (one saturated
1-slot model with a deferred queue of 2 plus a large 86-slot pool at
27/87 busy: the decision is a scale-up for the saturated model at every
target from 0% to 100%, the aggregate check over non-substitutable units
is refused at every threshold, and the mask is bounded at
~1.1 percentage points).
