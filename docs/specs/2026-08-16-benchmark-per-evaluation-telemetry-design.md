# Per-evaluation performance telemetry — design

**Date:** 2026-08-16
**Status:** Implementation specification (amendment)
**Repo:** berlinguyinca/autospec

**Amends:** [`2026-08-16-multi-model-engineering-team-design.md`](2026-08-16-multi-model-engineering-team-design.md)
— specifically §8 (capability evidence levels), §25 (required per-dispatch telemetry),
§26 (outcome statistics), §28 (model performance ledger) and §32 (calibration).

## Benchmark authority

`2026-08-16-repository-derived-real-work-benchmark-design.md` is authoritative for
the benchmark subsystem as a whole — corpus, replay methodology, scoring,
qualification rules, the `autospec bench` CLI surface, and `crates/autospec-bench/`.

This document is the **metric layer** beneath it: it deepens that spec's §31
(required inference telemetry) with the token-weighted throughput rule, the
context/performance curves, and the correct-work productivity metrics. Where the
two describe the same CLI verb or metric name, the RealWork spec wins.

## Scope binding

The parent spec makes *calibrated* an evidence level between *discovered* and
*observed*, but does not say what a calibration run measures. This amendment
defines that: the benchmark harness (`autospec bench`) and the performance
telemetry every evaluation must emit alongside its quality result.

Two constraints inherited from the parent spec apply here and are not restated
at each requirement below:

- **One ledger.** Benchmark telemetry extends the existing append-only routing
  ledger contract (parent §28) rather than introducing a second store. Raw
  invocation records nest under it; they do not fork it.
- **Unknown over fabricated.** Any metric the runtime or hardware cannot supply
  is recorded `null`/`unknown` (parent §25). A missing power reading is never
  interpolated, and a hardware metric from a different machine is never reused.

Metric names here map onto the parent's per-dispatch fields where they overlap
(`prompt_tok_s`, `decode_tok_s`, `ttft_ms`, `wall_clock_ms`, `input_tokens`,
`output_tokens`, `cached_tokens`); this amendment adds the benchmark-only
dimensions (preprocessing time, peak memory, power, per-call retention,
context curves) rather than renaming the shared ones.

## Requirement

Every benchmark evaluation must collect performance telemetry alongside its quality result.

Tokens-per-second measurements must not be limited to synthetic throughput tests.

This allows AutoSpec to answer questions such as:

- How fast was Q8 while solving repository tasks?
- Does Q6 become slower as reasoning complexity increases?
- How does W4A16 compare with Q8 on the exact same coding task?
- What happens to generation speed at 100K or 150K context?
- Does a faster model require more reasoning tokens to reach the same answer?
- Which configuration completes correct work fastest?

---

## Required Per-Evaluation Metrics

For every model invocation record:

```yaml
input_tokens:
output_tokens:

prompt_processing_seconds:
generation_seconds:
total_inference_seconds:
wall_clock_seconds:

prompt_tokens_per_second:
generation_tokens_per_second:

time_to_first_token_ms:

peak_gpu_memory_mb:
peak_host_memory_mb:

average_gpu_power_w:
peak_gpu_power_w:
```

Record unavailable hardware metrics as `null` rather than fabricating values.

---

## Evaluation-Level Metrics

Every individual benchmark task must report at minimum:

```text
quality result
pass/fail
input tokens
output tokens
prompt tokens/sec
generation tokens/sec
time to first token
total inference time
total task wall-clock time
```

Example:

```yaml
task: AS-017
category: debugging

result:
  passed: true
  score: 1.0

tokens:
  input: 48291
  output: 2847

performance:
  prompt_tps: 1142.7
  generation_tps: 91.4
  ttft_ms: 42680
  inference_seconds: 73.4
  wall_clock_seconds: 91.8
```

---

## Multi-Turn / Agentic Evaluations

For agentic AutoSpec tasks, report both per-call and whole-task statistics.

Example:

```yaml
task: AS-032

calls:
  - call: 1
    input_tokens: 18342
    output_tokens: 1921
    generation_tps: 96.2

  - call: 2
    input_tokens: 22418
    output_tokens: 843
    generation_tps: 94.8

  - call: 3
    input_tokens: 25103
    output_tokens: 1162
    generation_tps: 93.1
```

Also calculate task-level totals:

```yaml
aggregate:
  calls: 3

  input_tokens: 65863
  output_tokens: 3926

  average_generation_tps: 94.7
  median_generation_tps: 94.8
  p95_generation_tps: 96.2

  total_inference_seconds: 84.1
  total_wall_clock_seconds: 126.4

  passed: true
```

The implementation must distinguish between simple arithmetic averages and token-weighted throughput.

---

## Token-Weighted Throughput

For comparisons, calculate generation throughput as:

```text
total generated tokens
----------------------
total generation time
```

Do not simply average individual request tokens/sec values.

The same rule applies to prompt-processing throughput.

Both values may be retained, but token-weighted aggregate throughput is the authoritative comparison metric.

---

## Benchmark Category Reports

Every benchmark category must show performance alongside quality.

Example:

| Evaluation | Score | Prompt tok/s | Generation tok/s | Output tokens | Wall time |
|---|---:|---:|---:|---:|---:|
| HumanEval+ | 94.2% | 1,380 | 97.4 | 41,220 | 12m |
| MBPP+ | 91.7% | 1,420 | 98.1 | 68,440 | 19m |
| AutoSpec Implementation | 92.0% | 1,105 | 91.2 | 82,140 | 31m |
| AutoSpec Review | 96.0% | 1,086 | 87.3 | 48,210 | 24m |
| Long Context 100K | 94.0% | 903 | 83.1 | 21,440 | 18m |
| Long Context 150K | 91.0% | 811 | 76.8 | 19,830 | 25m |

Values above are illustrative only.

---

## Context Performance Curves

Long-context evaluations must explicitly report performance as context grows.

Required context points where supported:

```text
1K
10K
32K
64K
100K
128K
150K
190K
```

For each point record:

```text
quality score
prompt tok/s
generation tok/s
TTFT
peak memory
```

This allows the benchmark report to identify cases where a configuration technically supports 190K context but becomes operationally impractical.

---

## Correct-Work Performance

Add the following derived metrics:

### Seconds per successful task

```text
total wall-clock seconds / successful tasks
```

### Generation tokens per successful task

```text
generated tokens / successful tasks
```

### Inference seconds per successful task

```text
inference seconds / successful tasks
```

### Energy per successful task

Where power telemetry is available:

```text
Wh / successful task
```

These are important because raw tokens/sec does not necessarily represent useful productivity.

A model producing 100 tok/s but requiring four repair iterations may be less effective than a model producing 70 tok/s that solves the task correctly on its first attempt.

---

## Quality-Adjusted Performance

The report should present quality and throughput separately and may additionally calculate derived productivity metrics.

Do not collapse quality and tokens/sec into a single opaque number.

Recommended primary comparison:

```text
Pass rate
Generation tok/s
Seconds per successful task
```

Example:

| Candidate | Pass | Gen tok/s | Successful task time |
|---|---:|---:|---:|
| 4090 W4A16 | 91% | **102** | **42 sec** |
| 4090 Q6 | 95% | 48 | 71 sec |
| M4 Q8 | **97%** | 24 | 128 sec |

This immediately exposes the actual tradeoff.

---

## Hardware-Normalized Reporting

Every performance result must identify the hardware that produced it.

Never combine performance measurements across machines without identifying the machine.

Example:

```text
Qwen3.8 Q8 / M4 48GB
Qwen3.8 W4A16 / RTX 4090 24GB
```

Quality scores may be compared across hardware when the model/runtime behavior is otherwise equivalent.

Performance results are hardware-specific.

---

## Runtime Telemetry Adapter

Implement a common telemetry interface so benchmark code does not depend directly on vLLM, llama.cpp, MLX, or another runtime.

Conceptually:

```text
InferenceTelemetry

  prompt_tokens
  generated_tokens

  prompt_duration
  generation_duration

  prompt_tps
  generation_tps

  ttft

  memory
  power
```

Runtime adapters translate their native metrics into this common schema.

---

## Raw Results

Never discard raw invocation telemetry after aggregation.

Store:

```text
benchmark run
  -> evaluation
      -> task
          -> model invocation
```

This allows later analysis without rerunning expensive benchmarks.

---

## Dashboard Requirements

The benchmark dashboard must allow plotting:

```text
Quality vs generation tok/s
Quality vs time per successful task
Context size vs generation tok/s
Context size vs prompt tok/s
Context size vs quality
Concurrency vs aggregate tok/s
Quantization vs quality
Quantization vs memory
```

Users must be able to filter by:

```text
model
quant
runtime
hardware
context
benchmark
benchmark version
date
```

---

## Candidate Comparison

A comparison command such as:

```text
autospec bench compare qwen38-4090-w4 qwen38-m4-q8
```

should produce both quality and performance results.

Example summary:

```text
                     4090 W4       M4 Q8
------------------------------------------------
AutoSpec score         92.1%        96.4%
Review score           93.0%        97.1%
100K context score     94.2%        96.8%

Generation             101 t/s       27 t/s
Prompt 100K            912 t/s      481 t/s

Successful task         48 sec       119 sec
Output/success         3.8K          3.1K
```

Values are illustrative.

---

## Acceptance Criteria

The benchmark telemetry implementation is complete when:

- Every evaluation records input and output token counts.
- Every evaluation reports generation tokens/sec.
- Every evaluation reports prompt-processing tokens/sec where measurable.
- TTFT is recorded.
- Individual model calls are retained.
- Multi-call tasks have aggregated throughput.
- Performance is associated with exact hardware/runtime configuration.
- Long-context evaluations produce context/performance curves.
- Quality and performance appear together in reports.
- Raw telemetry is persisted.
- Correct-work productivity metrics are calculated.
- Candidate comparisons include both quality and speed.