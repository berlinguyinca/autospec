# Cluster: benchmark-and-outsourcing

Scope: benchmark-overfit gate + outsourced-implementation gate. Both detect
"app passes the spec, but only against the exact harness/dataset the spec
described" patterns.

Inputs:
- Benchmark configs + datasets.
- Generated app's implementation entry points.
- External service / model invocations.

Responsibilities:
- Run benchmark-overfit detection (see SKILL.md
  `## Benchmark leak and overfit gate`).
- Run outsourced-implementation gate (see SKILL.md
  `## Outsourced implementation gate`) — catch cases where the model passes
  off an LLM call as "implementation".

Output JSON shape:
```json
{
  "cluster": "benchmark-and-outsourcing",
  "category": "benchmark_overfit|outsourced_to_llm|hardcoded_answer",
  "file": "src/foo.ts:42",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category failing_test`).

TODO: backfill from `## Benchmark leak and overfit gate` +
`## Outsourced implementation gate` sections of SKILL.md.
